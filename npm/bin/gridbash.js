#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const http = require("node:http");
const https = require("node:https");
const path = require("node:path");
const { targetFor, targetKey } = require("./platforms.js");

const DEFAULT_UPDATE_CHECK_TIMEOUT_MS = 900;
const DEFAULT_UPDATE_CHECK_URL =
  "https://api.github.com/repos/jasonsuhari/gridbash/releases/latest";

function fail(message) {
  console.error(`gridbash: ${message}`);
  process.exit(1);
}

function packageRoot() {
  return path.resolve(__dirname, "..", "..");
}

function readPackageVersion(root = packageRoot()) {
  try {
    return JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
  } catch {
    return undefined;
  }
}

function tasklistImageName(output) {
  const match = /^\s*"([^"]+)"/m.exec(String(output || ""));
  return match?.[1];
}

function profileForProcessName(processName) {
  const name = path.win32.basename(String(processName || "").trim()).toLowerCase();
  switch (name) {
    case "powershell.exe":
      return "powershell";
    case "pwsh.exe":
      return "pwsh";
    case "cmd.exe":
      return "cmd";
    case "bash.exe":
    case "sh.exe":
    case "git-bash.exe":
      return "git-bash";
    default:
      return undefined;
  }
}

function detectInvokingProfile({
  platform = process.platform,
  parentPid = process.ppid,
  run = spawnSync,
} = {}) {
  if (platform !== "win32") {
    return undefined;
  }

  let result;
  try {
    result = run(
      "tasklist.exe",
      ["/FI", `PID eq ${parentPid}`, "/FO", "CSV", "/NH"],
      {
        encoding: "utf8",
        windowsHide: true,
      },
    );
  } catch {
    return undefined;
  }
  if (result.error || result.status !== 0) {
    return undefined;
  }

  return profileForProcessName(tasklistImageName(result.stdout));
}

function environmentForLaunch(env = process.env, invokingProfile = detectInvokingProfile()) {
  const childEnv = { ...env };
  if (
    invokingProfile &&
    !childEnv.GRIDBASH_PROFILE &&
    !childEnv.GRIDBASH_INVOKING_PROFILE
  ) {
    childEnv.GRIDBASH_INVOKING_PROFILE = invokingProfile;
  }
  return childEnv;
}

function parseVersion(value) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/.exec(value || "");
  if (!match) {
    return undefined;
  }

  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease: match[4],
  };
}

function comparePrerelease(left, right) {
  if (!left && !right) {
    return 0;
  }
  if (!left) {
    return 1;
  }
  if (!right) {
    return -1;
  }

  const leftParts = left.split(".");
  const rightParts = right.split(".");
  const count = Math.max(leftParts.length, rightParts.length);
  for (let index = 0; index < count; index += 1) {
    const leftPart = leftParts[index];
    const rightPart = rightParts[index];
    if (leftPart === undefined) {
      return -1;
    }
    if (rightPart === undefined) {
      return 1;
    }
    if (leftPart === rightPart) {
      continue;
    }

    const leftNumber = /^\d+$/.test(leftPart) ? Number(leftPart) : undefined;
    const rightNumber = /^\d+$/.test(rightPart) ? Number(rightPart) : undefined;
    if (leftNumber !== undefined && rightNumber !== undefined) {
      return Math.sign(leftNumber - rightNumber);
    }
    if (leftNumber !== undefined) {
      return -1;
    }
    if (rightNumber !== undefined) {
      return 1;
    }
    return leftPart < rightPart ? -1 : 1;
  }

  return 0;
}

function compareVersions(left, right) {
  const parsedLeft = parseVersion(left);
  const parsedRight = parseVersion(right);
  if (!parsedLeft || !parsedRight) {
    return 0;
  }

  for (const key of ["major", "minor", "patch"]) {
    if (parsedLeft[key] !== parsedRight[key]) {
      return Math.sign(parsedLeft[key] - parsedRight[key]);
    }
  }

  return comparePrerelease(parsedLeft.prerelease, parsedRight.prerelease);
}

function updateCheckTimeoutMs(env = process.env) {
  const value = Number.parseInt(env.GRIDBASH_UPDATE_CHECK_TIMEOUT_MS || "", 10);
  if (!Number.isFinite(value)) {
    return DEFAULT_UPDATE_CHECK_TIMEOUT_MS;
  }

  return Math.max(0, Math.min(5000, value));
}

function shouldSkipUpdateCheck(args, env = process.env, stderr = process.stderr) {
  if (env.GRIDBASH_NO_UPDATE_CHECK || env.GRIDBASH_UPDATE_CHECK === "0") {
    return true;
  }
  if (!stderr.isTTY) {
    return true;
  }

  return args.some((arg) => ["--help", "-h", "--version", "-V", "--mcp"].includes(arg));
}

function fetchLatestRelease(url, timeoutMs) {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (value) => {
      if (!settled) {
        settled = true;
        resolve(value);
      }
    };

    let parsedUrl;
    try {
      parsedUrl = new URL(url);
    } catch {
      finish(undefined);
      return;
    }

    if (!["http:", "https:"].includes(parsedUrl.protocol)) {
      finish(undefined);
      return;
    }

    const transport = parsedUrl.protocol === "http:" ? http : https;
    const request = transport.get(
      parsedUrl,
      {
        headers: {
          Accept: "application/vnd.github+json",
          "User-Agent": `gridbash/${readPackageVersion() || "unknown"}`,
        },
      },
      (response) => {
        if (!response.statusCode || response.statusCode < 200 || response.statusCode >= 300) {
          response.resume();
          finish(undefined);
          return;
        }

        let raw = "";
        response.setEncoding("utf8");
        response.on("data", (chunk) => {
          raw += chunk;
          if (raw.length > 128 * 1024) {
            request.destroy();
            finish(undefined);
          }
        });
        response.on("end", () => {
          try {
            const body = JSON.parse(raw);
            const tagName = body.tag_name || body.name;
            if (!tagName) {
              finish(undefined);
              return;
            }

            finish({
              tagName,
              version: tagName.replace(/^v/, ""),
              url: body.html_url || DEFAULT_UPDATE_CHECK_URL,
            });
          } catch {
            finish(undefined);
          }
        });
      },
    );

    request.on("error", () => finish(undefined));
    request.setTimeout(timeoutMs, () => {
      request.destroy();
      finish(undefined);
    });
  });
}

async function checkForUpdate(currentVersion, env = process.env) {
  if (!currentVersion) {
    return undefined;
  }

  const latest = await fetchLatestRelease(
    env.GRIDBASH_UPDATE_CHECK_URL || DEFAULT_UPDATE_CHECK_URL,
    updateCheckTimeoutMs(env),
  );
  if (!latest || compareVersions(latest.version, currentVersion) <= 0) {
    return undefined;
  }

  return latest;
}

function formatUpdateNotice(currentVersion, latest) {
  return [
    `gridbash: update available ${latest.tagName} (current v${currentVersion})`,
    `gridbash: see ${latest.url}`,
  ].join("\n");
}

async function maybePrintUpdateNotice(args, env = process.env, stderr = process.stderr) {
  if (shouldSkipUpdateCheck(args, env, stderr)) {
    return;
  }

  const currentVersion = readPackageVersion();
  const latest = await checkForUpdate(currentVersion, env);
  if (latest) {
    stderr.write(`${formatUpdateNotice(currentVersion, latest)}\n`);
  }
}

async function main() {
  const args = process.argv.slice(2);
  const target = targetFor();
  if (!target) {
    fail(`unsupported platform: ${targetKey(process.platform, process.arch)}`);
  }

  const exe = path.join(__dirname, target.directory, target.executable);
  const normalizedBinDir = path.resolve(__dirname).toLowerCase();
  const sourceManifest = path.resolve(__dirname, "..", "..", "Cargo.toml");

  if (
    (normalizedBinDir.includes(`${path.sep}.worktrees${path.sep}`.toLowerCase()) ||
      fs.existsSync(sourceManifest)) &&
    process.env.GRIDBASH_ALLOW_WORKTREE_LINK !== "1"
  ) {
    fail(
      'global command resolves to a source checkout. Run "npm run install:local" from the intended checkout, or set GRIDBASH_ALLOW_WORKTREE_LINK=1 to override.',
    );
  }

  if (!fs.existsSync(exe)) {
    fail(`missing packaged binary at ${exe}. Run "npm run build" from the gridbash repo.`);
  }

  await maybePrintUpdateNotice(args);

  const result = spawnSync(exe, args, {
    cwd: process.cwd(),
    env: environmentForLaunch(),
    stdio: "inherit",
    windowsHide: false,
  });

  if (result.error) {
    fail(result.error.message);
  }

  if (result.signal) {
    process.kill(process.pid, result.signal);
  } else {
    process.exit(result.status ?? 0);
  }
}

if (require.main === module) {
  main().catch((error) => fail(error.message));
} else {
  module.exports = {
    checkForUpdate,
    compareVersions,
    detectInvokingProfile,
    environmentForLaunch,
    fetchLatestRelease,
    formatUpdateNotice,
    profileForProcessName,
    shouldSkipUpdateCheck,
    tasklistImageName,
    updateCheckTimeoutMs,
  };
}
