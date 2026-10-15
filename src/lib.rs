use chrono::{Datelike, Duration, NaiveDate, Utc};
use rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedCommit {
    pub date: NaiveDate,
    pub timestamp: String,
    pub message: String,
}

fn default_commits_per_day() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FillPlan {
    pub version: u32,
    pub plan_id: String,
    pub created_at: String,
    pub repository: String,
    pub branch: String,
    pub starting_head: Option<String>,
    pub actor: String,
    pub author_email: String,
    pub source: String,
    pub timezone: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub fill_percent: u8,
    pub seed: u64,
    #[serde(default = "default_commits_per_day")]
    pub commits_per_day: u32,
    pub eligible_days: usize,
    pub selected_days: usize,
    pub commits: Vec<PlannedCommit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ApplyState {
    version: u32,
    plan_id: String,
    repository: String,
    applied_commits: Vec<String>,
    #[serde(default)]
    plan: Option<FillPlan>,
    #[serde(default)]
    pending: bool,
}

#[derive(Debug)]
pub enum PlanError {
    InvalidRange,
    InvalidFillPercent,
    InvalidCommitCount,
    InvalidWeekday(u32),
    Git(String),
    GitHub(String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRange => write!(f, "the end date must be on or after the start date"),
            Self::InvalidFillPercent => write!(f, "fill percentage must be between 0 and 100"),
            Self::InvalidCommitCount => write!(f, "commits per day must be at least 1"),
            Self::InvalidWeekday(day) => write!(f, "weekday must be between 1 and 7, got {day}"),
            Self::Git(message) => write!(f, "git: {message}"),
            Self::GitHub(message) => write!(f, "github: {message}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PlanError {}

impl From<std::io::Error> for PlanError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PlanError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub fn local_activity_dates(repository: &Path) -> Result<BTreeSet<NaiveDate>, PlanError> {
    let author_email = configured_author_email(repository)?;
    let repository_check = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(repository)
        .output()?;
    if !repository_check.status.success() {
        let message = String::from_utf8_lossy(&repository_check.stderr)
            .trim()
            .to_owned();
        return Err(PlanError::Git(if message.is_empty() {
            "the path is not a Git repository".to_owned()
        } else {
            message
        }));
    }

    let output = Command::new("git")
        .args(["log", "--all", "--format=%aI%x09%ae"])
        .current_dir(repository)
        .output()?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if message.contains("does not have any commits yet") {
            return Ok(BTreeSet::new());
        }
        return Err(PlanError::Git(if message.is_empty() {
            "could not read repository history".to_owned()
        } else {
            message
        }));
    }

    let mut dates = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((timestamp, email)) = line.trim().split_once('\t') else {
            continue;
        };
        if email != author_email {
            continue;
        }
        if timestamp.is_empty() {
            continue;
        }
        let date = chrono::DateTime::parse_from_rfc3339(timestamp)
            .map_err(|error| PlanError::Git(format!("invalid commit timestamp: {error}")))?
            .date_naive();
        dates.insert(date);
    }
    Ok(dates)
}

fn configured_author_email(repository: &Path) -> Result<String, PlanError> {
    let output = Command::new("git")
        .args(["config", "--get", "user.email"])
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        return Err(PlanError::Git(
            "configure git user.email before creating a local plan".to_owned(),
        ));
    }
    let email = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if email.is_empty() {
        return Err(PlanError::Git(
            "configure git user.email before creating a local plan".to_owned(),
        ));
    }
    Ok(email)
}

fn repository_state(repository: &Path) -> Result<(String, Option<String>), PlanError> {
    let branch = git_output(repository, &["symbolic-ref", "--short", "HEAD"])?;
    let head = git_output_optional(repository, &["rev-parse", "--verify", "HEAD"])?;
    Ok((branch, head))
}

fn git_output(repository: &Path, args: &[&str]) -> Result<String, PlanError> {
    Ok(String::from_utf8_lossy(&git_output_raw(repository, args)?)
        .trim()
        .to_owned())
}

fn git_output_raw(repository: &Path, args: &[&str]) -> Result<Vec<u8>, PlanError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(PlanError::Git(if message.is_empty() {
            format!("command failed: git {}", args.join(" "))
        } else {
            message
        }));
    }
    Ok(output.stdout)
}

fn git_output_optional(repository: &Path, args: &[&str]) -> Result<Option<String>, PlanError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository)
        .output()?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ));
    }
    Ok(None)
}

fn repository_is_clean(repository: &Path, ignored_path: &Path) -> Result<bool, PlanError> {
    let status = git_output_raw(
        repository,
        &["status", "--porcelain=v1", "--untracked-files=all", "-z"],
    )?;
    let repository_path = repository
        .canonicalize()
        .unwrap_or_else(|_| repository.to_path_buf());
    let ignored_path = ignored_path
        .canonicalize()
        .unwrap_or_else(|_| ignored_path.to_path_buf());
    let ignored_relative_path = ignored_path
        .strip_prefix(&repository_path)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"));
    for entry in String::from_utf8_lossy(&status)
        .split('\0')
        .filter(|entry| !entry.is_empty())
    {
        let path = entry.get(3..).unwrap_or(entry).replace('\\', "/");
        if !entry.starts_with("?? ") || ignored_relative_path.as_deref() != Some(path.as_str()) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn validate_inputs(
    start: NaiveDate,
    end: NaiveDate,
    fill_percent: u8,
    commits_per_day: u32,
    weekdays: &[u32],
) -> Result<(), PlanError> {
    if end < start {
        return Err(PlanError::InvalidRange);
    }
    if fill_percent > 100 {
        return Err(PlanError::InvalidFillPercent);
    }
    if commits_per_day == 0 {
        return Err(PlanError::InvalidCommitCount);
    }
    for &weekday in weekdays {
        if !(1..=7).contains(&weekday) {
            return Err(PlanError::InvalidWeekday(weekday));
        }
    }
    Ok(())
}

pub fn github_activity_dates(
    username: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<BTreeSet<NaiveDate>, PlanError> {
    let query = "query($login:String!, $from:DateTime!, $to:DateTime!) { user(login:$login) { contributionsCollection(from:$from, to:$to) { contributionCalendar { weeks { contributionDays { date contributionCount } } } } } }";
    let from = format!("{}T00:00:00Z", start.format("%Y-%m-%d"));
    let to = format!("{}T23:59:59Z", end.format("%Y-%m-%d"));
    let output = Command::new("gh")
        .arg("api")
        .arg("graphql")
        .arg("-f")
        .arg(format!("query={query}"))
        .arg("-f")
        .arg(format!("login={username}"))
        .arg("-f")
        .arg(format!("from={from}"))
        .arg("-f")
        .arg(format!("to={to}"))
        .output()?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(PlanError::GitHub(if message.is_empty() {
            "gh could not read the contribution calendar".to_owned()
        } else {
            message
        }));
    }

    parse_github_calendar(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_github_calendar(json: &str) -> Result<BTreeSet<NaiveDate>, PlanError> {
    let root: Value = serde_json::from_str(json)?;
    if let Some(errors) = root.get("errors").and_then(Value::as_array) {
        let message = errors
            .iter()
            .filter_map(|error| error.get("message").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(PlanError::GitHub(if message.is_empty() {
            "the GraphQL request returned an error".to_owned()
        } else {
            message
        }));
    }

    let weeks = root
        .pointer("/data/user/contributionsCollection/contributionCalendar/weeks")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            PlanError::GitHub("the response did not contain a contribution calendar".to_owned())
        })?;
    let mut dates = BTreeSet::new();
    for week in weeks {
        let Some(days) = week.get("contributionDays").and_then(Value::as_array) else {
            continue;
        };
        for day in days {
            let count = day
                .get("contributionCount")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if count == 0 {
                continue;
            }
            let date = day
                .get("date")
                .and_then(Value::as_str)
                .ok_or_else(|| PlanError::GitHub("a contribution day had no date".to_owned()))?
                .parse::<NaiveDate>()
                .map_err(|error| {
                    PlanError::GitHub(format!("invalid contribution date: {error}"))
                })?;
            dates.insert(date);
        }
    }
    Ok(dates)
}

pub fn make_plan(
    repository: &Path,
    start: NaiveDate,
    end: NaiveDate,
    fill_percent: u8,
    seed: u64,
    commits_per_day: u32,
    weekdays: &[u32],
) -> Result<FillPlan, PlanError> {
    validate_inputs(start, end, fill_percent, commits_per_day, weekdays)?;
    let author_email = configured_author_email(repository)?;
    let occupied = local_activity_dates(repository)?;
    let actor = author_email.clone();
    make_plan_from_activity(
        repository,
        start,
        end,
        fill_percent,
        seed,
        commits_per_day,
        weekdays,
        occupied,
        actor,
        author_email,
        "local-repository".to_owned(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn make_github_plan(
    repository: &Path,
    username: &str,
    start: NaiveDate,
    end: NaiveDate,
    fill_percent: u8,
    seed: u64,
    commits_per_day: u32,
    weekdays: &[u32],
) -> Result<FillPlan, PlanError> {
    validate_inputs(start, end, fill_percent, commits_per_day, weekdays)?;
    let author_email = configured_author_email(repository)?;
    let occupied = github_activity_dates(username, start, end)?;
    make_plan_from_activity(
        repository,
        start,
        end,
        fill_percent,
        seed,
        commits_per_day,
        weekdays,
        occupied,
        username.to_owned(),
        author_email,
        format!("github-profile:{username}"),
    )
}

#[allow(clippy::too_many_arguments)]
fn make_plan_from_activity(
    repository: &Path,
    start: NaiveDate,
    end: NaiveDate,
    fill_percent: u8,
    seed: u64,
    commits_per_day: u32,
    weekdays: &[u32],
    occupied: BTreeSet<NaiveDate>,
    actor: String,
    author_email: String,
    source: String,
) -> Result<FillPlan, PlanError> {
    let (branch, starting_head) = repository_state(repository)?;
    let repository_path = repository
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(repository));
    let mut eligible = Vec::new();
    let mut date = start;
    while date <= end {
        let weekday = date.weekday().number_from_monday();
        if !occupied.contains(&date) && (weekdays.is_empty() || weekdays.contains(&weekday)) {
            eligible.push(date);
        }
        date += Duration::days(1);
    }

    let selected_count = (eligible.len() * usize::from(fill_percent) + 50) / 100;
    let mut selected = eligible.clone();
    let mut rng = StdRng::seed_from_u64(seed);
    selected.shuffle(&mut rng);
    selected.truncate(selected_count);
    selected.sort_unstable();

    let commits = selected
        .iter()
        .flat_map(|date| {
            (0..commits_per_day).map(move |index| PlannedCommit {
                date: *date,
                timestamp: format!("{date}T12:00:00+00:00"),
                message: if commits_per_day == 1 {
                    format!("chore: contribution for {date}")
                } else {
                    format!(
                        "chore: contribution for {date} ({}/{commits_per_day})",
                        index + 1
                    )
                },
            })
        })
        .collect::<Vec<_>>();

    let created_at = Utc::now().to_rfc3339();
    let plan_id = format!(
        "{seed:016x}-{}-{}",
        start.format("%Y%m%d"),
        end.format("%Y%m%d")
    );
    Ok(FillPlan {
        version: 1,
        plan_id,
        created_at,
        repository: repository_path.display().to_string(),
        branch,
        starting_head,
        actor,
        author_email,
        source,
        timezone: "UTC".to_owned(),
        start,
        end,
        fill_percent,
        seed,
        commits_per_day,
        eligible_days: eligible.len(),
        selected_days: selected.len(),
        commits,
    })
}

pub fn save_plan(plan: &FillPlan, path: &Path) -> Result<(), PlanError> {
    let json = serde_json::to_string_pretty(plan)?;
    std::fs::write(path, format!("{json}\n"))?;
    Ok(())
}

pub fn load_plan(path: &Path) -> Result<FillPlan, PlanError> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

pub fn apply_plan(plan: &FillPlan, state_path: &Path) -> Result<Vec<String>, PlanError> {
    if plan.version != 1 {
        return Err(PlanError::Git("unsupported plan version".to_owned()));
    }
    let repository = Path::new(&plan.repository);
    ensure_no_git_operation(repository)?;
    let state_path = absolute_state_path(state_path)?;
    ensure_state_is_untracked(repository, &state_path)?;
    let (branch, head) = repository_state(repository)?;
    if branch != plan.branch {
        return Err(PlanError::Git(format!(
            "plan targets branch {}, current branch is {branch}",
            plan.branch
        )));
    }
    let state = load_apply_state(&state_path)?;
    let applied_commits = match &state {
        Some(state) => {
            if !matches!(state.version, 1 | 2)
                || state.plan_id != plan.plan_id
                || state.repository != plan.repository
                || state.applied_commits.len() > plan.commits.len()
                || (state.version == 2 && state.plan.as_ref() != Some(plan))
            {
                return Err(PlanError::Git(
                    "application state does not belong to this plan".to_owned(),
                ));
            }
            state.applied_commits.clone()
        }
        None => Vec::new(),
    };
    let mut parent = plan.starting_head.as_deref();
    for (index, id) in applied_commits.iter().enumerate() {
        verify_applied_commit(plan, index, id, parent)?;
        parent = Some(id);
    }
    let mut commit_ids = applied_commits;
    let expected_head = commit_ids
        .last()
        .cloned()
        .or_else(|| plan.starting_head.clone());
    if head != expected_head {
        if state.as_ref().is_some_and(|state| state.pending)
            && let Some(head) = head.as_ref()
            && commit_ids.len() < plan.commits.len()
        {
            verify_applied_commit(plan, commit_ids.len(), head, expected_head.as_deref())?;
            commit_ids.push(head.clone());
        } else {
            return Err(PlanError::Git(if state.is_some() {
                "repository HEAD changed since the last saved application state".to_owned()
            } else {
                "repository HEAD changed since this plan was created".to_owned()
            }));
        }
    }
    if !repository_is_clean(repository, &state_path)? {
        return Err(PlanError::Git(
            "working tree must be clean before applying a plan".to_owned(),
        ));
    }
    if configured_author_email(repository)? != plan.author_email {
        return Err(PlanError::Git(
            "configured git user.email changed since this plan was created".to_owned(),
        ));
    }

    let mut checkpoint = ApplyState {
        version: 2,
        plan_id: plan.plan_id.clone(),
        repository: plan.repository.clone(),
        applied_commits: commit_ids.clone(),
        plan: Some(plan.clone()),
        pending: false,
    };
    // Prove the checkpoint can be written before touching repository history.
    save_apply_state(&state_path, &checkpoint)?;
    for (index, commit) in plan.commits.iter().enumerate().skip(commit_ids.len()) {
        checkpoint.pending = true;
        save_apply_state(&state_path, &checkpoint)?;
        let output = Command::new("git")
            .args(["commit", "--allow-empty", "-m"])
            .arg(&commit.message)
            .env("GIT_AUTHOR_DATE", &commit.timestamp)
            .env("GIT_COMMITTER_DATE", &commit.timestamp)
            .env("GIT_AUTHOR_EMAIL", &plan.author_email)
            .env("GIT_COMMITTER_EMAIL", &plan.author_email)
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_COMMITTER_NAME")
            .current_dir(repository)
            .output()?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let reason = if message.is_empty() {
                "git commit failed"
            } else {
                message.as_str()
            };
            return Err(PlanError::Git(format!(
                "commit {}/{} failed: {}",
                index + 1,
                plan.commits.len(),
                reason
            )));
        }
        let id = git_output(repository, &["rev-parse", "HEAD"])?;
        let parent = commit_ids.last().or(plan.starting_head.as_ref());
        verify_applied_commit(plan, index, &id, parent.map(String::as_str))?;
        commit_ids.push(id);
        checkpoint.applied_commits = commit_ids.clone();
        checkpoint.pending = false;
        save_apply_state(&state_path, &checkpoint)?;
    }
    Ok(commit_ids)
}

fn ensure_no_git_operation(repository: &Path) -> Result<(), PlanError> {
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        let path = git_output(repository, &["rev-parse", "--git-path", marker])?;
        if repository.join(path).exists() {
            return Err(PlanError::Git(
                "finish or abort the current merge, rebase, cherry-pick, or revert before applying"
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn absolute_state_path(path: &Path) -> Result<PathBuf, PlanError> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| PlanError::Git("state path must name a file".to_owned()))?;
    Ok(parent.canonicalize()?.join(name))
}

fn ensure_state_is_untracked(repository: &Path, state_path: &Path) -> Result<(), PlanError> {
    if let Ok(relative) = state_path.strip_prefix(repository.canonicalize()?) {
        let output = Command::new("git")
            .args(["--literal-pathspecs", "ls-files", "--error-unmatch", "--"])
            .arg(relative)
            .current_dir(repository)
            .output()?;
        if output.status.success() {
            return Err(PlanError::Git(
                "application state must not be a tracked file".to_owned(),
            ));
        }
    }
    Ok(())
}

fn verify_applied_commit(
    plan: &FillPlan,
    index: usize,
    id: &str,
    parent: Option<&str>,
) -> Result<(), PlanError> {
    if !matches!(id.len(), 40 | 64) || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PlanError::Git(
            "application state contains an invalid commit id".to_owned(),
        ));
    }
    let repository = Path::new(&plan.repository);
    let raw = git_output_raw(
        repository,
        &[
            "show",
            "--no-patch",
            "--no-show-signature",
            "--format=%P%x00%T%x00%ae%x00%ce%x00%at%x00%ct%x00%B",
            id,
            "--",
        ],
    )?;
    let text = String::from_utf8_lossy(&raw);
    let fields: Vec<_> = text.splitn(7, '\0').collect();
    let commit = &plan.commits[index];
    let timestamp = chrono::DateTime::parse_from_rfc3339(&commit.timestamp)
        .map_err(|error| PlanError::Git(format!("invalid planned timestamp: {error}")))?
        .timestamp()
        .to_string();
    let valid = fields.len() == 7
        && fields[0] == parent.unwrap_or("")
        && fields[2] == plan.author_email
        && fields[3] == plan.author_email
        && fields[4] == timestamp
        && fields[5] == timestamp
        && fields[6].trim_end_matches('\n') == commit.message.trim_end_matches('\n');
    let same_tree = if let Some(parent) = parent {
        let tree = git_output(repository, &["rev-parse", &format!("{parent}^{{tree}}")])?;
        fields.get(1).is_some_and(|value| *value == tree)
    } else {
        git_output_raw(repository, &["ls-tree", id])?.is_empty()
    };
    if !valid || !same_tree {
        return Err(PlanError::Git(format!(
            "recorded or pending commit {} does not match the saved plan and parent tree",
            index + 1,
        )));
    }
    Ok(())
}

fn load_apply_state(path: &Path) -> Result<Option<ApplyState>, PlanError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(serde_json::from_str(&contents)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn save_apply_state(path: &Path, state: &ApplyState) -> Result<(), PlanError> {
    use std::io::Write;
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    let json = serde_json::to_string_pretty(state)?;
    writeln!(temporary, "{json}")?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn empty_repository() -> TempDir {
        let directory = tempfile::tempdir().expect("temporary directory");
        let status = Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(directory.path())
            .status()
            .expect("git init");
        assert!(status.success());
        for (key, value) in [
            ("user.name", "Fill It Up Test"),
            ("user.email", "fill-it-up-test@example.com"),
        ] {
            let status = Command::new("git")
                .args(["config", key, value])
                .current_dir(directory.path())
                .status()
                .expect("git config");
            assert!(status.success());
        }
        let status = Command::new("git")
            .args(["config", "commit.gpgsign", "false"])
            .current_dir(directory.path())
            .status()
            .expect("git config");
        assert!(status.success());
        git_output(
            directory.path(),
            &["config", "core.hooksPath", ".git/hooks"],
        )
        .unwrap();
        directory
    }

    fn test_repository() -> TempDir {
        let directory = empty_repository();
        let status = Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "baseline",
            ])
            .env("GIT_AUTHOR_DATE", "2026-01-02T12:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2026-01-02T12:00:00+00:00")
            .current_dir(directory.path())
            .status()
            .expect("git commit");
        assert!(status.success());
        directory
    }

    fn short_plan(repository: &Path) -> FillPlan {
        make_plan(
            repository,
            NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
            100,
            42,
            1,
            &[],
        )
        .unwrap()
    }

    fn checkpoint(plan: &FillPlan, ids: Vec<String>, pending: bool) -> ApplyState {
        ApplyState {
            version: 2,
            plan_id: plan.plan_id.clone(),
            repository: plan.repository.clone(),
            applied_commits: ids,
            plan: Some(plan.clone()),
            pending,
        }
    }

    fn create_planned_commit(plan: &FillPlan, index: usize) -> String {
        let commit = &plan.commits[index];
        let output = Command::new("git")
            .args(["commit", "--allow-empty", "-m", &commit.message])
            .env("GIT_AUTHOR_DATE", &commit.timestamp)
            .env("GIT_COMMITTER_DATE", &commit.timestamp)
            .current_dir(&plan.repository)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        git_output(Path::new(&plan.repository), &["rev-parse", "HEAD"]).unwrap()
    }

    #[test]
    fn applies_to_an_empty_branch_and_preserves_timestamps() {
        let repository = empty_repository();
        let state = tempfile::tempdir().unwrap();
        let plan = short_plan(repository.path());
        assert_eq!(plan.starting_head, None);
        let ids = apply_plan(&plan, &state.path().join("state.json")).unwrap();
        assert_eq!(ids.len(), 3);
        let dates = git_output(
            repository.path(),
            &["show", "-s", "--format=%aI%n%cI", &ids[0]],
        )
        .unwrap();
        let expected = chrono::DateTime::parse_from_rfc3339(&plan.commits[0].timestamp).unwrap();
        for date in dates.lines() {
            assert_eq!(
                chrono::DateTime::parse_from_rfc3339(date).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn rejects_staged_files_without_committing_them() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        fs::write(
            repository.path().join("private.txt"),
            "must stay out of commits",
        )
        .unwrap();
        git_output(repository.path(), &["add", "private.txt"]).unwrap();
        let error = apply_plan(&plan, &repository.path().join("state.json")).unwrap_err();
        assert!(error.to_string().contains("working tree must be clean"));
        assert_eq!(
            repository_state(repository.path()).unwrap().1,
            plan.starting_head
        );
        assert_eq!(
            git_output(repository.path(), &["diff", "--cached", "--name-only"]).unwrap(),
            "private.txt"
        );
    }

    #[test]
    fn rejects_changed_head_branch_and_author() {
        for change in ["head", "branch", "author"] {
            let repository = test_repository();
            let plan = short_plan(repository.path());
            match change {
                "head" => {
                    git_output(
                        repository.path(),
                        &["commit", "--allow-empty", "-m", "unrelated"],
                    )
                    .unwrap();
                }
                "branch" => {
                    git_output(repository.path(), &["checkout", "-b", "different"]).unwrap();
                }
                _ => {
                    git_output(
                        repository.path(),
                        &["config", "user.email", "changed@example.com"],
                    )
                    .unwrap();
                }
            }
            let before = repository_state(repository.path()).unwrap();
            assert!(apply_plan(&plan, &repository.path().join("state.json")).is_err());
            assert_eq!(before, repository_state(repository.path()).unwrap());
        }
    }

    #[test]
    fn rejects_an_in_progress_git_operation() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        fs::create_dir(repository.path().join(".git/rebase-merge")).unwrap();
        let error = apply_plan(&plan, &repository.path().join("state.json")).unwrap_err();
        assert!(error.to_string().contains("finish or abort"));
        assert_eq!(
            repository_state(repository.path()).unwrap().1,
            plan.starting_head
        );
    }

    #[test]
    fn rejects_an_unwritable_checkpoint_before_creating_commits() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        let state_directory = tempfile::tempdir().unwrap();
        // Persisting a checkpoint over a directory fails on every supported OS.
        assert!(apply_plan(&plan, state_directory.path()).is_err());
        assert_eq!(
            repository_state(repository.path()).unwrap().1,
            plan.starting_head
        );
    }

    #[test]
    fn rejects_a_modified_plan_even_when_its_id_matches() {
        let repository = test_repository();
        let mut plan = short_plan(repository.path());
        let state = repository.path().join("state.json");
        apply_plan(&plan, &state).unwrap();
        plan.commits[0].message = "different plan with the same id".to_owned();
        let error = apply_plan(&plan, &state).unwrap_err();
        assert!(error.to_string().contains("does not belong"));
    }

    #[test]
    fn resumes_after_a_git_failure_without_duplicates() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        let state = repository.path().join("state.json");
        let hook = repository.path().join(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\nif test \"$(git log -1 --format=%s)\" = \"chore: contribution for 2026-01-03\"; then\n  echo 'intentional test failure' >&2\n  exit 1\nfi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let error = apply_plan(&plan, &state).unwrap_err();
        assert!(error.to_string().contains("intentional test failure"));
        let progress = load_apply_state(&state).unwrap().unwrap();
        assert_eq!(progress.applied_commits.len(), 1);
        assert!(progress.pending);
        fs::remove_file(hook).unwrap();
        let ids = apply_plan(&plan, &state).unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], progress.applied_commits[0]);
        assert_eq!(
            git_output(repository.path(), &["rev-list", "--count", "HEAD"]).unwrap(),
            "4"
        );
        assert_eq!(ids, apply_plan(&plan, &state).unwrap());
    }

    #[test]
    fn recovers_a_commit_created_before_its_checkpoint_was_saved() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        let state = repository.path().join("state.json");
        save_apply_state(&state, &checkpoint(&plan, Vec::new(), true)).unwrap();
        let first = create_planned_commit(&plan, 0);
        let ids = apply_plan(&plan, &state).unwrap();
        assert_eq!(ids[0], first);
        assert_eq!(ids.len(), 3);
        assert_eq!(
            git_output(repository.path(), &["rev-list", "--count", "HEAD"]).unwrap(),
            "4"
        );
    }

    #[test]
    fn pending_recovery_rejects_an_unrelated_commit() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        let state = repository.path().join("state.json");
        save_apply_state(&state, &checkpoint(&plan, Vec::new(), true)).unwrap();
        git_output(
            repository.path(),
            &["commit", "--allow-empty", "-m", "unrelated"],
        )
        .unwrap();
        let before = repository_state(repository.path()).unwrap();
        let error = apply_plan(&plan, &state).unwrap_err();
        assert!(error.to_string().contains("does not match"));
        assert_eq!(repository_state(repository.path()).unwrap(), before);
    }

    #[test]
    fn pending_recovery_rejects_a_commit_that_changes_files() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        let state = repository.path().join("state.json");
        save_apply_state(&state, &checkpoint(&plan, Vec::new(), true)).unwrap();
        fs::write(
            repository.path().join("unexpected.txt"),
            "not an empty commit",
        )
        .unwrap();
        git_output(repository.path(), &["add", "unexpected.txt"]).unwrap();
        let id = create_planned_commit(&plan, 0);
        let error = apply_plan(&plan, &state).unwrap_err();
        assert!(error.to_string().contains("does not match"));
        assert_eq!(repository_state(repository.path()).unwrap().1, Some(id));
    }

    #[test]
    fn applying_preserves_existing_tracked_content() {
        let repository = test_repository();
        fs::write(
            repository.path().join("existing.txt"),
            "keep this content\n",
        )
        .unwrap();
        git_output(repository.path(), &["add", "existing.txt"]).unwrap();
        git_output(repository.path(), &["commit", "-m", "existing content"]).unwrap();
        let plan = short_plan(repository.path());
        let original_tree = git_output(repository.path(), &["rev-parse", "HEAD^{tree}"]).unwrap();
        apply_plan(&plan, &repository.path().join("state.json")).unwrap();
        assert_eq!(
            git_output(repository.path(), &["rev-parse", "HEAD^{tree}"]).unwrap(),
            original_tree
        );
        assert_eq!(
            fs::read_to_string(repository.path().join("existing.txt")).unwrap(),
            "keep this content\n"
        );
    }

    #[test]
    fn validates_every_recorded_parent_before_resuming() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        let state = repository.path().join("state.json");
        let ids = apply_plan(&plan, &state).unwrap();
        let mut progress = load_apply_state(&state).unwrap().unwrap();
        progress.applied_commits[0] = ids[1].clone();
        save_apply_state(&state, &progress).unwrap();
        let error = apply_plan(&plan, &state).unwrap_err();
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn upgrades_a_legacy_checkpoint_after_verifying_its_commits() {
        let repository = test_repository();
        let plan = short_plan(repository.path());
        let state = repository.path().join("state.json");
        let first = create_planned_commit(&plan, 0);
        let mut legacy = checkpoint(&plan, vec![first.clone()], false);
        legacy.version = 1;
        legacy.plan = None;
        save_apply_state(&state, &legacy).unwrap();
        assert_eq!(apply_plan(&plan, &state).unwrap()[0], first);
        let upgraded = load_apply_state(&state).unwrap().unwrap();
        assert_eq!(upgraded.version, 2);
        assert_eq!(upgraded.plan, Some(plan));
    }

    #[test]
    fn rejects_a_tracked_state_file() {
        let repository = test_repository();
        let state = repository.path().join("state.json");
        fs::write(&state, "{}").unwrap();
        git_output(repository.path(), &["add", "state.json"]).unwrap();
        git_output(repository.path(), &["commit", "-m", "tracked state"]).unwrap();
        let plan = short_plan(repository.path());
        let error = apply_plan(&plan, &state).unwrap_err();
        assert!(error.to_string().contains("must not be a tracked file"));
        assert_eq!(fs::read_to_string(state).unwrap(), "{}");
    }

    #[test]
    fn plans_a_seeded_subset_of_empty_days() {
        let repository = test_repository();
        let plan = make_plan(
            repository.path(),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            50,
            42,
            1,
            &[],
        )
        .unwrap();

        assert_eq!(plan.eligible_days, 9);
        assert_eq!(plan.selected_days, 5);
        assert!(!plan.commits.iter().any(|commit| commit.date.day() == 2));
        assert_eq!(plan.commits[0].timestamp.len(), 25);
    }

    #[test]
    fn the_same_seed_produces_the_same_plan() {
        let first_repository = test_repository();
        let second_repository = test_repository();
        let start = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();
        let first = make_plan(first_repository.path(), start, end, 25, 7, 1, &[]).unwrap();
        let second = make_plan(second_repository.path(), start, end, 25, 7, 1, &[]).unwrap();

        assert_eq!(first.commits, second.commits);
    }

    #[test]
    fn plans_can_round_trip_through_json() {
        let repository = test_repository();
        let plan = make_plan(
            repository.path(),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            100,
            1,
            1,
            &[],
        )
        .unwrap();
        let path = repository.path().join("plan.json");
        save_plan(&plan, &path).unwrap();
        let loaded = load_plan(&path).unwrap();
        assert_eq!(plan, loaded);
        assert!(fs::metadata(path).unwrap().is_file());
    }

    #[test]
    fn applies_a_plan_with_backdated_commits() {
        let repository = test_repository();
        let plan = make_plan(
            repository.path(),
            NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 4).unwrap(),
            100,
            1,
            1,
            &[],
        )
        .unwrap();

        let state_path = repository.path().join("plan.state.json");
        let commit_ids = apply_plan(&plan, &state_path).unwrap();
        assert_eq!(commit_ids.len(), 2);
        assert_eq!(
            git_output(repository.path(), &["rev-list", "--count", "HEAD"]).unwrap(),
            "3"
        );
        let repeated_ids = apply_plan(&plan, &state_path).unwrap();
        assert_eq!(repeated_ids, commit_ids);
        assert_eq!(
            git_output(repository.path(), &["rev-list", "--count", "HEAD"]).unwrap(),
            "3"
        );
        fs::write(repository.path().join("unrelated.txt"), "changed").unwrap();
        let error = apply_plan(&plan, &state_path).unwrap_err();
        assert!(error.to_string().contains("working tree must be clean"));
    }

    #[test]
    fn supports_multiple_commits_per_selected_day() {
        let repository = test_repository();
        let plan = make_plan(
            repository.path(),
            NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(),
            100,
            1,
            2,
            &[],
        )
        .unwrap();

        assert_eq!(plan.selected_days, 1);
        assert_eq!(plan.commits.len(), 2);
        assert!(plan.commits[1].message.contains("(2/2)"));
    }

    #[test]
    fn parses_github_calendar_days_with_contributions() {
        let json = r#"{
          "data": {
            "user": {
              "contributionsCollection": {
                "contributionCalendar": {
                  "weeks": [
                    {"contributionDays": [
                      {"date": "2026-01-01", "contributionCount": 2},
                      {"date": "2026-01-02", "contributionCount": 0}
                    ]}
                  ]
                }
              }
            }
          }
        }"#;

        let dates = parse_github_calendar(json).unwrap();
        assert_eq!(dates.len(), 1);
        assert!(dates.contains(&NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()));
    }

    #[test]
    fn surfaces_github_graphql_errors() {
        let json = r#"{"errors":[{"message":"Could not resolve to a User"}]}"#;
        let error = parse_github_calendar(json).unwrap_err();
        assert!(error.to_string().contains("Could not resolve to a User"));
    }
}
