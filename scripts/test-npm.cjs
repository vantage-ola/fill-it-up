#!/usr/bin/env node

// Pack and install a real native build. All caches and installations stay in tmp.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const root = path.resolve(__dirname, "..");
const executable = process.platform === "win32" ? "fill-it-up.exe" : "fill-it-up";
const binary = path.resolve(root, process.argv[2] || path.join("target", "debug", executable));
assert.ok(fs.existsSync(binary), `Build the CLI first or pass a compiled binary: ${binary}`);
const npmCli = process.env.npm_execpath;
assert.ok(npmCli, "Run this check with npm --prefix npm run test:package -- [binary]");
assert.equal(fs.readFileSync(path.join(root, "npm", "LICENSE"), "utf8"),
  fs.readFileSync(path.join(root, "LICENSE"), "utf8"), "npm and native licenses must match");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "fill-it-up npm check "));
const env = { ...process.env,
  npm_config_cache: path.join(temporary, "cache"),
  npm_config_offline: "true",
  npm_config_update_notifier: "false",
  npm_config_audit: "false",
  npm_config_fund: "false",
};
delete env.FILL_IT_UP_BINARY;

function run(command, args, options = {}) {
  const { status = 0, ...rest } = options;
  const result = spawnSync(command, args, {
    cwd: temporary, env, encoding: "utf8", timeout: 120_000, ...rest,
  });
  assert.ifError(result.error);
  assert.equal(result.status, status,
    `${command} ${args.join(" ")}\n${result.stdout}\n${result.stderr}`);
  return result;
}
function npm(args, options) { return run(process.execPath, [npmCli, ...args], options); }
function launch(launcher, args, options) { return run(process.execPath, [launcher, ...args], options); }

try {
  const staging = path.join(temporary, "package source");
  fs.mkdirSync(path.join(staging, "bin"), { recursive: true });
  for (const filename of ["package.json", "README.md", "LICENSE"]) {
    fs.copyFileSync(path.join(root, "npm", filename), path.join(staging, filename));
  }
  const stagedLauncher = path.join(staging, "bin", "fill-it-up.js");
  fs.copyFileSync(path.join(root, "npm", "bin", "fill-it-up.js"), stagedLauncher);
  fs.chmodSync(stagedLauncher, 0o755);

  const missing = launch(stagedLauncher, ["--version"], { status: 1 });
  assert.match(missing.stderr, /no native binary/);
  const nativeDirectory = path.join(staging, "dist", `${process.platform}-${process.arch}`);
  fs.mkdirSync(nativeDirectory, { recursive: true });
  fs.copyFileSync(binary, path.join(nativeDirectory, executable));
  fs.chmodSync(path.join(nativeDirectory, executable), 0o755);
  const badOverride = launch(stagedLauncher, ["--version"], {
    status: 1, env: { ...env, FILL_IT_UP_BINARY: path.join(temporary, "missing binary") },
  });
  assert.match(badOverride.stderr, /FILL_IT_UP_BINARY does not exist/);

  const metadata = JSON.parse(npm(["pack", "--json", "--ignore-scripts", "--pack-destination", temporary],
    { cwd: staging }).stdout)[0];
  const archive = path.join(temporary, metadata.filename);
  assert.ok(metadata.files.some(({ path: filename }) => filename.endsWith(`/${executable}`)));
  assert.ok(metadata.files.some(({ path: filename }) => filename === "LICENSE"));
  const packageJson = JSON.parse(fs.readFileSync(path.join(staging, "package.json")));
  const version = packageJson.version;
  if (process.env.GITHUB_REF_TYPE === "tag") {
    assert.equal(process.env.GITHUB_REF_NAME, `v${version}`, "release tag and package version must match");
  }
  const expectedVersion = `fill-it-up ${version}`;
  assert.equal(run(binary, ["--version"]).stdout.trim(), expectedVersion);

  const local = path.join(temporary, "local install");
  fs.mkdirSync(local);
  fs.writeFileSync(path.join(local, "package.json"), '{"name":"package-check","private":true}\n');
  npm(["install", "--ignore-scripts", "--no-package-lock", archive], { cwd: local });
  const installedLauncher = path.join(local, "node_modules", ...packageJson.name.split("/"), "bin", "fill-it-up.js");
  assert.equal(launch(installedLauncher, ["--version"]).stdout.trim(), expectedVersion);
  launch(installedLauncher, ["--not-a-real-option"], { status: 2 });

  // npm exec is the modern npx entry point and exercises the installed bin shim.
  assert.equal(npm(["exec", "--offline", "--", "fill-it-up", "--version"], { cwd: local })
    .stdout.trim(), expectedVersion);
  const fresh = path.join(temporary, "fresh npx directory");
  fs.mkdirSync(fresh);
  assert.equal(npm(["exec", "--offline", "--yes", `--package=${archive}`, "--", "fill-it-up", "--version"],
    { cwd: fresh }).stdout.trim(), expectedVersion);

  const prefix = path.join(temporary, "global prefix");
  npm(["install", "--global", "--prefix", prefix, "--ignore-scripts", archive]);
  const globalBin = process.platform === "win32" ? prefix : path.join(prefix, "bin");
  const globalCommand = process.platform === "win32" ? "fill-it-up.cmd" : "fill-it-up";
  assert.ok(fs.existsSync(path.join(globalBin, globalCommand)));
  if (process.platform !== "win32") {
    assert.equal(run(path.join(prefix, "bin", "fill-it-up"), ["--version"]).stdout.trim(), expectedVersion);
  }
  // Resolve the real global shim through PATH. An absolute quoted command is
  // parsed again by Windows cmd.exe and can lose quotes when the prefix has spaces.
  // Exercise the same lookup on every OS, including failure-code forwarding.
  const pathKey = Object.keys(env).find((key) => key.toLowerCase() === "path") || "PATH";
  const globalEnv = { ...env, [pathKey]: globalBin + path.delimiter + (env[pathKey] || "") };
  fs.writeFileSync(path.join(fresh, "package.json"), JSON.stringify({
    private: true, scripts: {
      check: "fill-it-up --version",
      "check-error": "fill-it-up --not-a-real-option",
    },
  }));
  assert.equal(npm(["run", "--silent", "check"], { cwd: fresh, env: globalEnv })
    .stdout.trim(), expectedVersion);
  npm(["run", "--silent", "check-error"], { cwd: fresh, env: globalEnv, status: 2 });

  const repository = path.join(temporary, "contribution repo with spaces");
  const planPath = path.join(temporary, "saved plan.json");
  launch(installedLauncher, ["init", repository]);
  for (const [key, value] of [
    ["user.name", "Package Test"], ["user.email", "package-test@example.com"],
    ["commit.gpgsign", "false"], ["core.hooksPath", ".git/hooks"],
  ]) { run("git", ["-C", repository, "config", key, value]); }
  launch(installedLauncher, ["plan", "--repo", repository, "--local-only", "--from", "2026-01-03",
    "--to", "2026-01-03", "--fill", "100", "--commits-per-day", "2", "--out", planPath]);
  const plan = JSON.parse(fs.readFileSync(planPath));
  assert.equal(plan.commits.length, 2);
  launch(installedLauncher, ["preview", planPath]);
  // Confirmation input must reach the native process through the Node launcher.
  launch(installedLauncher, ["apply", planPath], { input: "y\n" });
  const firstHead = run("git", ["-C", repository, "rev-parse", "HEAD"]).stdout;
  launch(installedLauncher, ["apply", planPath, "--yes"]);
  assert.equal(run("git", ["-C", repository, "rev-parse", "HEAD"]).stdout, firstHead);
  assert.equal(run("git", ["-C", repository, "rev-list", "--count", "HEAD"]).stdout.trim(), "2");
  console.log(`Passed: ${process.platform}-${process.arch}; packed local/global installs, fresh npx, paths with spaces, arguments, exit codes, confirmation, and plan/apply/reapply.`);
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
