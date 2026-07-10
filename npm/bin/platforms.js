const TARGETS = Object.freeze({
  "win32-x64": Object.freeze({
    platform: "win32",
    arch: "x64",
    directory: "win32-x64",
    executable: "gridbash.exe",
    cargoTarget: "x86_64-pc-windows-msvc",
  }),
  "linux-x64": Object.freeze({
    platform: "linux",
    arch: "x64",
    directory: "linux-x64",
    executable: "gridbash",
    cargoTarget: "x86_64-unknown-linux-gnu",
  }),
  "linux-arm64": Object.freeze({
    platform: "linux",
    arch: "arm64",
    directory: "linux-arm64",
    executable: "gridbash",
    cargoTarget: "aarch64-unknown-linux-gnu",
  }),
});

function targetKey(platform, arch) {
  return `${platform}-${arch}`;
}

function targetFor(platform = process.platform, arch = process.arch) {
  return TARGETS[targetKey(platform, arch)];
}

function supportedTargets() {
  return Object.values(TARGETS);
}

module.exports = {
  supportedTargets,
  targetFor,
  targetKey,
};
