use chrono::{Datelike, Local, NaiveDate};
use clap::{Parser, Subcommand};
use fill_it_up::{
    FillPlan, PlanError, apply_plan, load_plan, make_github_plan, make_plan, save_plan,
};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Debug, Parser)]
#[command(
    name = "fill-it-up",
    version,
    about = "Plan local commits for selected empty contribution days"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new local Git repository for contribution commits.
    Init { path: PathBuf },
    /// Preview and save a reproducible plan from local history or a GitHub profile.
    Plan {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Read contribution days from this GitHub username using `gh auth`.
        #[arg(long, conflicts_with = "local_only")]
        github: Option<String>,
        /// Read contribution days from the target repository's configured Git author.
        #[arg(long, conflicts_with = "github")]
        local_only: bool,
        /// Plan a calendar year. Cannot be combined with --from or --to.
        #[arg(long, conflicts_with_all = ["from", "to"])]
        year: Option<i32>,
        /// First date for an explicit planning range.
        #[arg(long, requires = "to", conflicts_with = "year")]
        from: Option<NaiveDate>,
        /// Last date for an explicit planning range.
        #[arg(long, requires = "from", conflicts_with = "year")]
        to: Option<NaiveDate>,
        #[arg(long, default_value_t = 35)]
        fill: u8,
        #[arg(long, default_value_t = 42)]
        seed: u64,
        #[arg(long, default_value_t = 1)]
        commits_per_day: u32,
        #[arg(long, value_delimiter = ',')]
        weekdays: Vec<u32>,
        #[arg(long, default_value = "fill-plan.json")]
        out: PathBuf,
    },
    /// Print a saved plan.
    Preview { plan: PathBuf },
    /// Apply a saved plan after validating the repository state.
    Apply {
        plan: PathBuf,
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), PlanError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => init_repository(&path),
        Command::Plan {
            repo,
            github,
            local_only: _,
            year,
            from,
            to,
            fill,
            seed,
            commits_per_day,
            weekdays,
            out,
        } => {
            let today = Local::now().date_naive();
            let (start, requested_end) = match (year, from, to) {
                (Some(year), None, None) => {
                    if year > today.year() {
                        return Err(PlanError::InvalidRange);
                    }
                    let start =
                        NaiveDate::from_ymd_opt(year, 1, 1).ok_or(PlanError::InvalidRange)?;
                    let end =
                        NaiveDate::from_ymd_opt(year, 12, 31).ok_or(PlanError::InvalidRange)?;
                    (start, end)
                }
                (None, Some(start), Some(end)) => (start, end),
                _ => return Err(PlanError::InvalidRange),
            };
            if start > today {
                return Err(PlanError::InvalidRange);
            }
            let end = requested_end.min(today);
            if end < start {
                return Err(PlanError::InvalidRange);
            }
            let plan = if let Some(username) = github {
                make_github_plan(
                    &repo,
                    &username,
                    start,
                    end,
                    fill,
                    seed,
                    commits_per_day,
                    &weekdays,
                )?
            } else {
                make_plan(&repo, start, end, fill, seed, commits_per_day, &weekdays)?
            };
            print_plan(&plan);
            save_plan(&plan, &out)?;
            println!("saved plan to {}", out.display());
            Ok(())
        }
        Command::Preview { plan } => {
            let plan = load_plan(&plan)?;
            print_plan(&plan);
            Ok(())
        }
        Command::Apply { plan, yes } => {
            let state_path = application_state_path(&plan);
            let plan = load_plan(&plan)?;
            print_plan(&plan);
            if !yes {
                eprint!("Apply these {} commits? [y/N] ", plan.commits.len());
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    println!("cancelled");
                    return Ok(());
                }
            }
            let commit_ids = apply_plan(&plan, &state_path)?;
            println!(
                "applied or verified {} commits; state saved to {}",
                commit_ids.len(),
                state_path.display()
            );
            Ok(())
        }
    }
}

fn application_state_path(plan: &Path) -> PathBuf {
    let parent = plan.parent().unwrap_or_else(|| std::path::Path::new("."));
    let stem = plan
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("fill-plan");
    parent.join(format!("{stem}.state.json"))
}

fn init_repository(path: &PathBuf) -> Result<(), PlanError> {
    std::fs::create_dir_all(path)?;
    let status = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(path)
        .status()?;
    if !status.success() {
        return Err(PlanError::Git("could not initialize repository".to_owned()));
    }
    println!("initialized repository at {}", path.display());
    Ok(())
}

fn print_plan(plan: &FillPlan) {
    println!("plan {}", plan.plan_id);
    println!("repository: {}", plan.repository);
    println!("actor: {}", plan.actor);
    println!("source: {}", plan.source);
    println!("range: {} to {}", plan.start, plan.end);
    println!("eligible empty days: {}", plan.eligible_days);
    println!(
        "selected days: {} ({}%)",
        plan.selected_days, plan.fill_percent
    );
    println!("commits per selected day: {}", plan.commits_per_day);
    if plan.commits.is_empty() {
        println!("no commits selected");
        return;
    }
    println!("dates:");
    for commit in &plan.commits {
        println!("  {} ({})", commit.date, commit.date.weekday());
    }
}
