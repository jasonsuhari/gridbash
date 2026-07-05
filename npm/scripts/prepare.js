const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..", "..");
const source = path.join(root, "target", "release", "gridbash.exe");
const outDir = path.join(root, "npm", "bin", "win32-x64");
const target = path.join(outDir, "gridbash.exe");

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

if (process.platform !== "win32" || process.arch !== "x64") {
  console.log("gridbash prepare: skipping binary build for non-win32-x64 platform");
  process.exit(0);
}

run(process.platform === "win32" ? "cargo.exe" : "cargo", ["build", "--release"]);
fs.mkdirSync(outDir, { recursive: true });
fs.copyFileSync(source, target);
console.log(`gridbash prepare: copied ${path.relative(root, target)}`);
