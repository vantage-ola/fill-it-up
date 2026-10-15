#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const executableName = process.platform === "win32" ? "fill-it-up.exe" : "fill-it-up";
const platformDirectory = `${process.platform}-${process.arch}`;
const packageRoot = path.join(__dirname, "..");
const candidates = [];

if (process.env.FILL_IT_UP_BINARY) {
  candidates.push(path.resolve(process.env.FILL_IT_UP_BINARY));
} else {
  candidates.push(path.join(packageRoot, "dist", platformDirectory, executableName));
  // Development fallback is available only from a source checkout.
  if (fs.existsSync(path.join(packageRoot, "..", "Cargo.toml"))) {
    candidates.push(
      path.join(packageRoot, "..", "target", "release", executableName),
      path.join(packageRoot, "..", "target", "debug", executableName),
    );
  }
}

const binary = candidates.find((candidate) => fs.existsSync(candidate));
if (!binary) {
  console.error(process.env.FILL_IT_UP_BINARY
    ? `FILL_IT_UP_BINARY does not exist: ${candidates[0]}`
    : `fill-it-up has no native binary for ${platformDirectory}. ` +
      "Install a platform release or set FILL_IT_UP_BINARY to a compiled binary.",
  );
  process.exitCode = 1;
} else {
  const result = spawnSync(binary, process.argv.slice(2), {
    stdio: "inherit",
    windowsHide: false,
  });

  if (result.error) {
    console.error(`could not start fill-it-up: ${result.error.message}`);
    process.exitCode = 1;
  } else if (result.signal) {
    process.kill(process.pid, result.signal);
  } else {
    process.exitCode = result.status ?? 1;
  }
}
