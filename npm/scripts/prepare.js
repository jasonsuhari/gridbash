const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { targetFor, targetKey } = require("../bin/platforms.js");

const root = path.resolve(__dirname, "..", "..");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit"
  });

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit code ${result.status}`);
  }
}

const platformTarget = targetFor();
if (!platformTarget) {
  console.log(
    `gridbash prepare: skipping unsupported platform ${targetKey(process.platform, process.arch)}`,
  );
  process.exit(0);
}

const executable = process.platform === "win32" ? "gridbash.exe" : "gridbash";
const source = path.join(root, "target", "release", executable);
const outDir = path.join(root, "npm", "bin", platformTarget.directory);
const packagedBinary = path.join(outDir, platformTarget.executable);

run(process.platform === "win32" ? "cargo.exe" : "cargo", [
  "build",
  "--release",
  "--bin",
  "gridbash",
]);
fs.mkdirSync(outDir, { recursive: true });
fs.copyFileSync(source, packagedBinary);
if (process.platform !== "win32") {
  fs.chmodSync(packagedBinary, 0o755);
}
console.log(`gridbash prepare: copied ${path.relative(root, packagedBinary)}`);
