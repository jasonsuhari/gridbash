const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { targetFor, targetKey } = require("../bin/platforms.js");

const root = path.resolve(__dirname, "..", "..");
const packageJson = require(path.join(root, "package.json"));

function fail(message) {
  console.error(`install-local: ${message}`);
  process.exit(1);
}

function run(command, args, options = {}) {
  const invocation = commandInvocation(command, args);
  const result = spawnSync(invocation.command, invocation.args, {
    cwd: root,
    stdio: options.capture ? "pipe" : "inherit",
    encoding: "utf8",
    shell: false,
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0 && !options.allowFailure) {
    const output = options.capture ? `\n${result.stderr || result.stdout}` : "";
    fail(`${command} ${args.join(" ")} failed with exit code ${result.status}${output}`);
  }

  return options.capture ? result.stdout.trim() : "";
}

function commandInvocation(command, args) {
  if (process.platform !== "win32" || !command.endsWith(".cmd")) {
    return { command, args };
  }

  return {
    command: process.env.ComSpec || "cmd.exe",
    args: ["/d", "/s", "/c", [command, ...args].map(quoteCmd).join(" ")],
  };
}

function quoteCmd(value) {
  if (!/[ \t"&|<>^]/.test(value)) {
    return value;
  }

  return `"${value.replace(/"/g, '\\"')}"`;
}

function npmCommand() {
  return process.platform === "win32" ? "npm.cmd" : "npm";
}

function assertNotLinked() {
  const globalRoot = run(npmCommand(), ["root", "-g"], { capture: true });
  const packagePath = path.join(globalRoot, packageJson.name);
  const stat = fs.lstatSync(packagePath);
  if (stat.isSymbolicLink()) {
    fail(`global install is still linked to a worktree: ${packagePath}`);
  }

  const real = fs.realpathSync(packagePath);
  if (real.startsWith(path.join(root, ".worktrees"))) {
    fail(`global install still resolves into .worktrees: ${real}`);
  }
  if (fs.existsSync(path.join(packagePath, "Cargo.toml"))) {
    fail(`global install still points at a source checkout: ${packagePath}`);
  }

  console.log(`install-local: installed copy at ${packagePath}`);
}

if (!targetFor()) {
  fail(`unsupported platform: ${targetKey(process.platform, process.arch)}`);
}

run("node", ["npm/scripts/prepare.js"]);

const packOutput = run(npmCommand(), ["pack", "--json", "--ignore-scripts"], { capture: true });
const pack = JSON.parse(packOutput)[0];
const tarball = path.join(root, pack.filename);

try {
  run(npmCommand(), ["uninstall", "-g", packageJson.name], { allowFailure: true });
  run(npmCommand(), ["install", "-g", tarball, "--no-audit", "--no-fund"]);
  assertNotLinked();
} finally {
  fs.rmSync(tarball, { force: true });
}
