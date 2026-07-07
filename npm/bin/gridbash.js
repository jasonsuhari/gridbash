#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

function fail(message) {
  console.error(`gridbash: ${message}`);
  process.exit(1);
}

if (process.platform !== "win32") {
  fail("this npm package currently ships the Windows x64 build only");
}

if (process.arch !== "x64") {
  fail(`unsupported architecture: ${process.arch}`);
}

const exe = path.join(__dirname, "win32-x64", "gridbash.exe");
const normalizedBinDir = path.resolve(__dirname).toLowerCase();
const sourceManifest = path.resolve(__dirname, "..", "..", "Cargo.toml");

if (
  (normalizedBinDir.includes(`${path.sep}.worktrees${path.sep}`.toLowerCase()) ||
    fs.existsSync(sourceManifest)) &&
  process.env.GRIDBASH_ALLOW_WORKTREE_LINK !== "1"
) {
  fail(
    'global command resolves to a source checkout. Run "npm run install:local" from the intended checkout, or set GRIDBASH_ALLOW_WORKTREE_LINK=1 to override.'
  );
}

if (!fs.existsSync(exe)) {
  fail(`missing packaged binary at ${exe}. Run "npm run build" from the gridbash repo.`);
}

const result = spawnSync(exe, process.argv.slice(2), {
  cwd: process.cwd(),
  stdio: "inherit",
  windowsHide: false
});

if (result.error) {
  fail(result.error.message);
}

if (result.signal) {
  process.kill(process.pid, result.signal);
} else {
  process.exit(result.status ?? 0);
}
