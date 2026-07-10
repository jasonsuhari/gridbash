const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { targetFor, targetKey } = require("../bin/platforms.js");

const root = path.resolve(__dirname, "..", "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const platformTarget = targetFor();

function fail(message) {
  throw new Error(`gridbash prepare: ${message}`);
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    fail(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}

function copyVersionedPlist(source, target) {
  const raw = fs.readFileSync(source, "utf8");
  fs.writeFileSync(target, raw.replaceAll("__GRIDBASH_VERSION__", packageJson.version));
}

function resetDirectory(target) {
  const relative = path.relative(root, target);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    fail(`refusing to reset path outside repository: ${target}`);
  }
  fs.rmSync(target, { recursive: true, force: true });
  fs.mkdirSync(target, { recursive: true });
}

function prepareBinary(packageDir) {
  const source = path.join(root, "target", "release", platformTarget.executable);
  const binDir = path.join(packageDir, "bin");
  const packagedBinary = path.join(binDir, platformTarget.executable);
  resetDirectory(binDir);
  fs.copyFileSync(source, packagedBinary);
  if (process.platform !== "win32") {
    fs.chmodSync(packagedBinary, 0o755);
  }
  if (process.platform === "linux") {
    const helperSource = path.join(root, "target", "release", "gridbash-voice");
    const helperTarget = path.join(binDir, "gridbash-voice");
    fs.copyFileSync(helperSource, helperTarget);
    fs.chmodSync(helperTarget, 0o755);
  }
}

function prepareMacos(packageDir) {
  const source = path.join(root, "target", "release", "gridbash");
  const app = path.join(packageDir, "GridBash.app");
  const contents = path.join(app, "Contents");
  const macosDir = path.join(contents, "MacOS");
  const helperContents = path.join(contents, "Helpers", "GridBashSpeech.app", "Contents");
  const helperMacosDir = path.join(helperContents, "MacOS");
  const helper = path.join(helperMacosDir, "gridbash-speech");
  const nativeSource = path.join(root, "native", "macos");
  const targetArch = process.arch === "arm64" ? "arm64" : "x86_64";

  resetDirectory(app);
  fs.mkdirSync(macosDir, { recursive: true });
  fs.mkdirSync(helperMacosDir, { recursive: true });
  fs.copyFileSync(source, path.join(macosDir, "gridbash"));
  copyVersionedPlist(path.join(nativeSource, "GridBash.Info.plist"), path.join(contents, "Info.plist"));
  copyVersionedPlist(
    path.join(nativeSource, "GridBashSpeech.Info.plist"),
    path.join(helperContents, "Info.plist"),
  );
  run("xcrun", [
    "swiftc",
    path.join(nativeSource, "GridBashSpeech.swift"),
    "-O",
    "-target",
    `${targetArch}-apple-macosx13.0`,
    "-framework",
    "Speech",
    "-framework",
    "AVFoundation",
    "-o",
    helper,
  ]);
  fs.chmodSync(path.join(macosDir, "gridbash"), 0o755);
  fs.chmodSync(helper, 0o755);
}

if (!platformTarget) {
  fail(`unsupported platform architecture: ${targetKey(process.platform, process.arch)}`);
}

const packageDir = path.join(root, "npm", "platforms", platformTarget.directory);
if (!fs.existsSync(path.join(packageDir, "package.json"))) {
  fail(`missing native package manifest for ${platformTarget.directory}`);
}

const cargoArgs = ["build", "--release"];
if (process.platform === "linux") {
  cargoArgs.push("--bins");
} else {
  cargoArgs.push("--bin", "gridbash");
}
run(process.platform === "win32" ? "cargo.exe" : "cargo", cargoArgs);

if (process.platform === "darwin") {
  prepareMacos(packageDir);
} else {
  prepareBinary(packageDir);
}

console.log(`gridbash prepare: assembled ${path.relative(root, packageDir)}`);
