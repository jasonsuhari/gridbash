const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..", "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const platformKey = `${process.platform}-${process.arch}`;
const packageDir = path.join(root, "npm", "platforms", platformKey);

function fail(message) {
  throw new Error(`gridbash prepare: ${message}`);
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
  });

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

function prepareWindows() {
  const source = path.join(root, "target", "release", "gridbash.exe");
  const binDir = path.join(packageDir, "bin");
  resetDirectory(binDir);
  fs.copyFileSync(source, path.join(binDir, "gridbash.exe"));
}

function prepareMacos() {
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

if (!fs.existsSync(path.join(packageDir, "package.json"))) {
  fail(`unsupported platform architecture: ${platformKey}`);
}

run(process.platform === "win32" ? "cargo.exe" : "cargo", ["build", "--release"]);

if (process.platform === "win32" && process.arch === "x64") {
  prepareWindows();
} else if (process.platform === "darwin" && ["arm64", "x64"].includes(process.arch)) {
  prepareMacos();
} else {
  fail(`unsupported platform architecture: ${platformKey}`);
}

console.log(`gridbash prepare: assembled ${path.relative(root, packageDir)}`);
