const fs = require("node:fs");
const path = require("node:path");

const projectRoot = path.resolve(__dirname, "..", "..");
const exactVersionPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

function assertExactVersion(version) {
  if (!exactVersionPattern.test(version)) {
    throw new Error(`unsupported version: ${version}`);
  }
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function nativeManifestPaths(root) {
  const platformRoot = path.join(root, "npm", "platforms");
  return fs
    .readdirSync(platformRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(platformRoot, entry.name, "package.json"))
    .filter((manifest) => fs.existsSync(manifest))
    .sort();
}

function setProjectVersion(root, version) {
  assertExactVersion(version);

  const packagePath = path.join(root, "package.json");
  const cargoPath = path.join(root, "Cargo.toml");
  const packageJson = readJson(packagePath);
  const cargoToml = fs.readFileSync(cargoPath, "utf8");
  const packageVersionPattern = /(\[package\][\s\S]*?^version\s*=\s*")[^"]+(".*$)/m;

  if (!packageVersionPattern.test(cargoToml)) {
    throw new Error("could not find Cargo.toml package version");
  }

  const manifests = nativeManifestPaths(root);
  if (manifests.length === 0) {
    throw new Error("no native npm package manifests found");
  }

  const nativePackages = manifests.map((manifest) => ({
    manifest,
    value: readJson(manifest),
  }));
  const optionalDependencies = packageJson.optionalDependencies || {};

  for (const nativePackage of nativePackages) {
    if (!nativePackage.value.name) {
      throw new Error(`native package is missing a name: ${nativePackage.manifest}`);
    }
    if (!(nativePackage.value.name in optionalDependencies)) {
      throw new Error(`root optionalDependencies is missing ${nativePackage.value.name}`);
    }
  }

  packageJson.version = version;
  for (const nativePackage of nativePackages) {
    optionalDependencies[nativePackage.value.name] = version;
    nativePackage.value.version = version;
  }
  packageJson.optionalDependencies = optionalDependencies;

  writeJson(packagePath, packageJson);
  fs.writeFileSync(
    cargoPath,
    cargoToml.replace(
      packageVersionPattern,
      (_match, before, after) => `${before}${version}${after}`,
    ),
  );
  for (const nativePackage of nativePackages) {
    writeJson(nativePackage.manifest, nativePackage.value);
  }

  return {
    packagePath,
    cargoPath,
    nativeManifests: manifests,
  };
}

function nightlyVersion(currentVersion, date, runNumber, commitSha) {
  assertExactVersion(currentVersion);
  if (!/^\d{8}$/.test(date)) {
    throw new Error(`nightly date must be YYYYMMDD: ${date}`);
  }
  if (!/^[1-9]\d*$/.test(String(runNumber))) {
    throw new Error(`nightly run number must be a positive integer: ${runNumber}`);
  }
  if (!/^[0-9a-f]{7,64}$/i.test(commitSha)) {
    throw new Error(`nightly commit must be a git SHA: ${commitSha}`);
  }

  const [coreVersion, prerelease] = currentVersion.split("-", 2);
  const [major, minor] = coreVersion.split(".").map(Number);
  const baseVersion = prerelease
    ? coreVersion
    : `${major}.${minor + 1}.0`;
  const shortSha = commitSha.slice(0, 12).toLowerCase();
  return `${baseVersion}-nightly.${date}.${runNumber}.g${shortSha}`;
}

function usage() {
  console.error(`Usage:
  node npm/scripts/version.js set <version>
  node npm/scripts/version.js nightly <YYYYMMDD> <run-number> <commit-sha>`);
}

if (require.main === module) {
  const [command, ...args] = process.argv.slice(2);
  try {
    if (command === "set" && args.length === 1) {
      setProjectVersion(projectRoot, args[0]);
      console.log(args[0]);
    } else if (command === "nightly" && args.length === 3) {
      const currentVersion = readJson(path.join(projectRoot, "package.json")).version;
      console.log(nightlyVersion(currentVersion, args[0], args[1], args[2]));
    } else {
      usage();
      process.exitCode = 1;
    }
  } catch (error) {
    console.error(`version: ${error.message}`);
    process.exitCode = 1;
  }
}

module.exports = {
  assertExactVersion,
  nightlyVersion,
  setProjectVersion,
};
