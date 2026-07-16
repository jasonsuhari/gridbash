const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const projectRoot = path.resolve(__dirname, "..", "..");
const defaultOwner = "jasonmatthewsuhari";
const defaultRepository = "jasonsuhari/gridbash";

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function expectedPackageNames(root) {
  const rootPackage = readJson(path.join(root, "package.json"));
  const dependencyNames = Object.keys(rootPackage.optionalDependencies || {});
  const platformRoot = path.join(root, "npm", "platforms");
  const nativeNames = fs
    .readdirSync(platformRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => readJson(path.join(platformRoot, entry.name, "package.json")).name)
    .sort();

  const dependencySet = new Set(dependencyNames);
  const nativeSet = new Set(nativeNames);
  const missingDependencies = nativeNames.filter((name) => !dependencySet.has(name));
  const missingManifests = dependencyNames.filter((name) => !nativeSet.has(name));

  if (missingDependencies.length || missingManifests.length) {
    const details = [
      missingDependencies.length
        ? `native manifests missing from optionalDependencies: ${missingDependencies.join(", ")}`
        : undefined,
      missingManifests.length
        ? `optionalDependencies missing native manifests: ${missingManifests.join(", ")}`
        : undefined,
    ]
      .filter(Boolean)
      .join("; ");
    throw new Error(details);
  }

  return [rootPackage.name, ...dependencyNames];
}

function commandInvocation(command, args) {
  if (process.platform !== "win32" || !command.endsWith(".cmd")) {
    return { command, args };
  }

  const quote = (value) =>
    /[ \t"&|<>^]/.test(value) ? `"${value.replace(/"/g, '\\"')}"` : value;
  return {
    command: process.env.ComSpec || "cmd.exe",
    args: ["/d", "/s", "/c", [command, ...args].map(quote).join(" ")],
  };
}

function runNpm(args) {
  const npm = process.platform === "win32" ? "npm.cmd" : "npm";
  const invocation = commandInvocation(npm, args);
  const result = spawnSync(invocation.command, invocation.args, {
    cwd: projectRoot,
    encoding: "utf8",
    shell: false,
  });

  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error((result.stderr || result.stdout || "npm command failed").trim());
  }
  return result.stdout.trim();
}

function parseOwners(output) {
  return output
    .split(/\r?\n/)
    .map((line) => /^(@?[^\s<]+)/.exec(line.trim())?.[1])
    .filter(Boolean);
}

function repositorySlug(repository) {
  let value = typeof repository === "string" ? repository : repository?.url;
  if (!value) {
    return undefined;
  }

  value = value.trim();
  if (value === "npm/security-holder") {
    return value;
  }

  return value
    .replace(/^git\+/, "")
    .replace(/^github:/, "")
    .replace(/^git@github\.com:/, "")
    .replace(/^ssh:\/\/git@github\.com\//, "")
    .replace(/^https?:\/\/github\.com\//, "")
    .replace(/\.git\/?$/, "")
    .replace(/\/$/, "")
    .toLowerCase();
}

function validatePackage({ expectedName, expectedOwner, expectedRepository, metadata, owners }) {
  const errors = [];
  if (metadata.name !== expectedName) {
    errors.push(`registry returned package name ${metadata.name || "<missing>"}`);
  }
  if (!owners.includes(expectedOwner)) {
    errors.push(
      `expected owner ${expectedOwner}; current owners: ${owners.join(", ") || "<none>"}`,
    );
  }

  const actualRepository = repositorySlug(metadata.repository);
  const expectedSlug = repositorySlug(expectedRepository);
  const reclaimedSecurityPlaceholder =
    metadata.version === "0.0.1-security" &&
    actualRepository === "npm/security-holder" &&
    owners.includes(expectedOwner);

  if (actualRepository !== expectedSlug && !reclaimedSecurityPlaceholder) {
    errors.push(
      `expected repository ${expectedSlug}; registry reports ${actualRepository || "<missing>"}`,
    );
  }

  return { errors, reclaimedSecurityPlaceholder };
}

function auditRegistry({
  root = projectRoot,
  expectedOwner = defaultOwner,
  expectedRepository = defaultRepository,
  npm = runNpm,
} = {}) {
  const reports = [];
  for (const name of expectedPackageNames(root)) {
    try {
      const metadata = JSON.parse(npm(["view", name, "name", "version", "repository", "--json"]));
      const owners = parseOwners(npm(["owner", "ls", name]));
      const result = validatePackage({
        expectedName: name,
        expectedOwner,
        expectedRepository,
        metadata,
        owners,
      });
      reports.push({ name, ...result });
    } catch (error) {
      reports.push({ name, errors: [error.message], reclaimedSecurityPlaceholder: false });
    }
  }
  return reports;
}

function main() {
  const reports = auditRegistry({
    expectedOwner: process.env.GRIDBASH_NPM_OWNER || defaultOwner,
    expectedRepository: process.env.GRIDBASH_NPM_REPOSITORY || defaultRepository,
  });
  let failed = false;

  for (const report of reports) {
    if (report.errors.length) {
      failed = true;
      console.error(`npm release preflight: ${report.name} failed`);
      for (const error of report.errors) {
        console.error(`  - ${error}`);
      }
    } else if (report.reclaimedSecurityPlaceholder) {
      console.log(
        `npm release preflight: ${report.name} is a reclaimed npm security placeholder; the first GridBash publish will replace its repository metadata.`,
      );
    } else {
      console.log(`npm release preflight: ${report.name} ownership and repository verified.`);
    }
  }

  if (failed) {
    process.exitCode = 1;
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  auditRegistry,
  expectedPackageNames,
  parseOwners,
  repositorySlug,
  validatePackage,
};
