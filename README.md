# fill-it-up

Plan and create local Git commits for selected empty days in a GitHub contribution calendar.

The project is currently in early development. The implementation starts as a Rust CLI. An npm package will later distribute the same compiled binary.

## Current status

The first working slice can initialize a repository, inspect dates belonging to the configured `git user.email`, create a seeded plan for a year or explicit date range, preview it, save it as JSON, and apply it after validating the repository state. It can also schedule multiple commits per selected day and read a GitHub profile calendar through `gh`.

```bash
cargo run -- init ./contributions
git -C ./contributions config user.name "YOUR_NAME"
git -C ./contributions config user.email "YOUR_GITHUB_EMAIL"
cargo run -- plan --repo ./contributions --year 2026 --fill 35 --seed 42 --out ./fill-plan.json
cargo run -- preview ./fill-plan.json
cargo run -- apply ./fill-plan.json
```

Use an explicit range instead of a calendar year when needed. `--commits-per-day`
defaults to one:

```bash
cargo run -- plan --repo ./contributions \
  --from 2026-01-01 --to 2026-03-31 \
  --fill 35 --commits-per-day 2 --out ./fill-plan.json
```

Planning requires `git user.email` to be configured because it will author the generated commits. Local plans use repository history and label their source as `local-repository`; they do not claim to know about activity in other repositories.

Profile planning uses the GitHub CLI's existing authentication and the contribution calendar API:

```bash
cargo run -- plan --repo ./contributions --github YOUR_HANDLE \
  --year 2026 --fill 35 --seed 42 --out ./fill-plan.json
```

The saved plan records whether its activity came from local history or a GitHub profile. Applying a plan requires the target branch, starting HEAD, configured author, and working tree to match the saved plan. A sidecar state file records completed commit IDs. Before resuming, the CLI checks each recorded commit's parent, dates, email, message, and unchanged file tree.

If a run stops, keep the original plan and its `.state.json` file, fix the reported issue, then run `apply` again with that same plan. The CLI can recover a commit created just before progress was saved. Editing or regenerating the plan after application has started is rejected. Older state files are upgraded after their recorded commits have been checked.

### Inspect and publish

After applying a plan, inspect the author and committer dates:

```bash
git -C ./contributions log --format=fuller
```

Create an empty GitHub repository you own, then add its URL and push the `main` branch created by `init`:

```bash
git -C ./contributions remote add origin https://github.com/YOUR_HANDLE/YOUR_REPOSITORY.git
git -C ./contributions push -u origin main
```

Use an email associated with your GitHub account. This one-day workflow was tested on macOS and confirmed by the user in `vantage-ola/disposable-fill`. CI is configured to run installation and workflow checks on other supported platforms.

### Development checks

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
npm --prefix npm run test:package
```

The npm check packs the native binary and installs it locally, into a temporary global prefix, and through the npm exec entry point used by `npx`. It runs a full plan/apply/reapply flow with paths containing spaces. All test repositories, installations, and npm caches stay in temporary directories. To test a release build, pass its path after `--`.

The npm package will be published as `@oosanya/fill-it-up` with the `fill-it-up` command. The release workflow bundles the supported native binaries, so npm users do not need Rust installed. Once published, run it with:

```bash
npx @oosanya/fill-it-up --help
```

For maintainers, automated npm publishing requires a repository Actions secret named `NPM_TOKEN` with npm publishing access. If it is missing, the workflow publishes the native release and reports that npm publishing was skipped. Keep the Cargo and npm versions in sync when creating a release tag.
