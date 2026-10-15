# @oosanya/fill-it-up

Plan and create local Git commits for selected empty contribution days. This package bundles the Rust CLI for macOS ARM64/x64, Linux x64, and Windows x64. Git and Node.js 18 or newer are required. GitHub profile planning also requires the GitHub CLI (`gh`) with an authenticated account.

After the package is published:

```bash
npx @oosanya/fill-it-up --help
```

Or install the command globally:

```bash
npm install -g @oosanya/fill-it-up
fill-it-up --help
```

See the [project README](https://github.com/vantage-ola/fill-it-up#readme) for planning, previewing, applying, and publishing commits.

## Development

This package is the Node entry point for the Rust `fill-it-up` binary. Release automation places the matching native binary in `dist/<platform>-<architecture>/` before publishing the package.

For local development, point the launcher at a compiled binary:

```bash
FILL_IT_UP_BINARY=../target/debug/fill-it-up node bin/fill-it-up.js --help
```

From the repository root, build and test a packed installation:

```bash
cargo build --locked
npm --prefix npm run test:package
```

This checks local and temporary global installations, fresh npm exec (`npx`), paths with spaces, arguments, exit codes, confirmation input, and a full plan/apply/reapply workflow. It uses temporary directories and an isolated npm cache. To select a release binary, pass its path after `--`.
