const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { setProjectVersion } = require("./version");

const root = path.resolve(__dirname, "..", "..");
const args = process.argv.slice(2);
const bump = args.find((arg) => !arg.startsWith("--"));
const flags = new Set(args.filter((arg) => arg.startsWith("--")));

function fail(message) {
  console.error(`release: ${message}`);
  process.exit(1);
}

function usage() {
  console.log(`Usage:
  npm run release -- patch --notes docs/devlogs/YYYY-MM-DD-title.md
  npm run release -- minor --notes docs/devlogs/YYYY-MM-DD-title.md --push --yes
  npm run release -- 1.2.3 --notes docs/devlogs/YYYY-MM-DD-title.md --push --yes

Options:
  --notes <path>       Devlog or release notes file to copy into docs/releases/vX.Y.Z.md
  --push               Push the release commit and tag to origin
  --yes                Required with --push
  --allow-branch       Allow releasing from a branch other than main or master
  --allow-unmerged-branches
                      Allow release even when origin has unmerged task branches
  --skip-checks        Skip cargo fmt, tests, prepare, and npm pack dry run
`);
}

function readFlag(name) {
  const index = args.indexOf(name);
  if (index === -1) {
    return undefined;
  }
  return args[index + 1];
}

function run(command, commandArgs, options = {}) {
  const invocation = commandInvocation(command, commandArgs);
  const result = spawnSync(invocation.command, invocation.args, {
    cwd: root,
    stdio: options.capture ? "pipe" : "inherit",
    encoding: "utf8",
    shell: false,
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    const detail = options.capture ? `\n${result.stderr || result.stdout}` : "";
    fail(`${command} ${commandArgs.join(" ")} failed with exit code ${result.status}${detail}`);
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

function readJson(file) {
  return JSON.parse(fs.readFileSync(path.join(root, file), "utf8"));
}

function npmCommand() {
  return process.platform === "win32" ? "npm.cmd" : "npm";
}

function parseVersion(value) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-.+)?$/.exec(value);
  if (!match) {
    fail(`unsupported version: ${value}`);
  }
  return match.slice(1).map(Number);
}

function nextVersion(current, requested) {
  if (/^\d+\.\d+\.\d+(?:-.+)?$/.test(requested)) {
    return requested;
  }

  const [major, minor, patch] = parseVersion(current);
  switch (requested) {
    case "major":
      return `${major + 1}.0.0`;
    case "minor":
      return `${major}.${minor + 1}.0`;
    case "patch":
      return `${major}.${minor}.${patch + 1}`;
    default:
      fail("first argument must be patch, minor, major, or an explicit x.y.z version");
  }
}

function latestDevlog() {
  const dir = path.join(root, "docs", "devlogs");
  if (!fs.existsSync(dir)) {
    return undefined;
  }

  return fs
    .readdirSync(dir)
    .filter((name) => /^\d{4}-\d{2}-\d{2}-.+\.md$/.test(name))
    .sort()
    .pop();
}

function releaseNotesPath(version) {
  const explicit = readFlag("--notes");
  const source = explicit || latestDevlog();
  if (!source) {
    fail("no notes found; run npm run devlog -- --title \"...\" or pass --notes <path>");
  }

  const sourcePath = path.resolve(root, source);
  if (!fs.existsSync(sourcePath)) {
    fail(`notes file does not exist: ${source}`);
  }

  const outDir = path.join(root, "docs", "releases");
  const outPath = path.join(outDir, `v${version}.md`);
  fs.mkdirSync(outDir, { recursive: true });

  const notes = fs.readFileSync(sourcePath, "utf8").trim();
  fs.writeFileSync(
    outPath,
    `# v${version}

Source notes: ${path.relative(root, sourcePath).replaceAll("\\", "/")}

${notes}
`,
  );

  return outPath;
}

function assertClean() {
  const status = run("git", ["status", "--porcelain"], { capture: true });
  if (status) {
    fail("working tree must be clean before release");
  }
}

function assertBranchAllowed() {
  const branch = run("git", ["branch", "--show-current"], { capture: true });
  if (!flags.has("--allow-branch") && branch !== "main" && branch !== "master") {
    fail(`release branch is ${branch}; use main/master or pass --allow-branch`);
  }
}

function hasRemote(name) {
  const result = spawnSync("git", ["remote", "get-url", name], {
    cwd: root,
    stdio: "ignore",
    shell: false,
  });
  return result.status === 0;
}

function refreshOriginBranches() {
  if (!hasRemote("origin")) {
    return;
  }

  const result = spawnSync(
    "git",
    ["fetch", "--prune", "origin", "+refs/heads/*:refs/remotes/origin/*"],
    {
      cwd: root,
      stdio: "pipe",
      encoding: "utf8",
      shell: false,
    },
  );

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    fail(
      `could not refresh origin branches before release\n${result.stderr || result.stdout}\nUse --allow-unmerged-branches only after manually reviewing branch state.`,
    );
  }
}

function taskBranchName(remoteBranch) {
  return remoteBranch.replace(/^origin\//, "");
}

function isTaskBranch(remoteBranch) {
  return /^(chore|docs|feat|fix|refactor|test)\//.test(taskBranchName(remoteBranch));
}

function isMergedIntoHead(remoteBranch) {
  const result = spawnSync("git", ["merge-base", "--is-ancestor", remoteBranch, "HEAD"], {
    cwd: root,
    stdio: "ignore",
    shell: false,
  });
  return result.status === 0;
}

function unmergedOriginTaskBranches() {
  if (!hasRemote("origin")) {
    return [];
  }

  const output = run("git", ["for-each-ref", "--format=%(refname:short)", "refs/remotes/origin"], {
    capture: true,
  });

  return output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .filter((branch) => branch !== "origin/HEAD")
    .filter((branch) => !["origin/main", "origin/master"].includes(branch))
    .filter(isTaskBranch)
    .filter((branch) => !isMergedIntoHead(branch));
}

function summarizeBranch(remoteBranch) {
  const latest = run("git", ["log", "--oneline", "-1", remoteBranch], { capture: true });
  return `- ${remoteBranch}: ${latest}`;
}

function assertNoUnmergedTaskBranches() {
  if (flags.has("--allow-unmerged-branches")) {
    return;
  }

  refreshOriginBranches();
  const branches = unmergedOriginTaskBranches();
  if (branches.length === 0) {
    return;
  }

  const visible = branches.slice(0, 12).map(summarizeBranch).join("\n");
  const hiddenCount = branches.length - 12;
  const hidden = hiddenCount > 0 ? `\n- ...and ${hiddenCount} more` : "";
  fail(
    `unmerged origin task branches exist; review, merge, or delete them before release:\n${visible}${hidden}\nUse --allow-unmerged-branches only after an explicit branch review.`,
  );
}

function assertTagFree(tag) {
  const result = spawnSync("git", ["rev-parse", "--verify", tag], {
    cwd: root,
    stdio: "ignore",
    shell: false,
  });
  if (result.status === 0) {
    fail(`tag already exists: ${tag}`);
  }
}

if (flags.has("--help") || flags.has("-h")) {
  usage();
  process.exit(0);
}

if (!bump) {
  usage();
  process.exit(1);
}

if (flags.has("--push") && !flags.has("--yes")) {
  fail("--push requires --yes");
}

assertClean();
assertBranchAllowed();
assertNoUnmergedTaskBranches();

const packageJson = readJson("package.json");
const cargoToml = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const cargoVersion = /^version = "([^"]+)"/m.exec(cargoToml)?.[1];
if (packageJson.version !== cargoVersion) {
  fail(`package.json version (${packageJson.version}) does not match Cargo.toml (${cargoVersion})`);
}

const version = nextVersion(packageJson.version, bump);
const tag = `v${version}`;
assertTagFree(tag);

console.log(`release: ${packageJson.version} -> ${version}`);

const { nativeManifests } = setProjectVersion(root, version);
const notesPath = releaseNotesPath(version);

run("cargo", ["check"]);

if (!flags.has("--skip-checks")) {
  run("cargo", ["fmt", "--check"]);
  run("cargo", ["clippy", "--", "-D", "warnings"]);
  run("cargo", ["test"]);
  run("node", ["npm/scripts/prepare.js"]);
  run(npmCommand(), ["pack", "--dry-run", "--ignore-scripts"]);
}

run("git", [
  "add",
  "Cargo.toml",
  "Cargo.lock",
  "package.json",
  ...nativeManifests.map((manifest) => path.relative(root, manifest)),
  path.relative(root, notesPath),
]);
run("git", ["commit", "-m", `chore: release ${tag}`]);
run("git", ["tag", "-a", tag, "-m", tag]);

if (flags.has("--push")) {
  run("git", ["push", "origin", "HEAD"]);
  run("git", ["push", "origin", tag]);
}

console.log(`release: created ${tag}`);
if (flags.has("--push")) {
  console.log("release: pushed release commit and tag; GitHub Actions will publish npm and GitHub release");
} else {
  console.log("release: local only; push the commit and tag when ready");
}
