const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const {
  expectedPackageNames,
  parseOwners,
  repositorySlug,
  validatePackage,
} = require("./release-preflight");

const root = path.resolve(__dirname, "..", "..");

test("expectedPackageNames covers the launcher and every native package", () => {
  assert.deepEqual(expectedPackageNames(root), [
    "gridbash",
    "gridbash-win32-x64",
    "gridbash-linux-x64",
    "gridbash-linux-arm64",
    "gridbash-darwin-arm64",
    "gridbash-darwin-x64",
  ]);
});

test("release workflow gates native builds on the registry preflight", () => {
  const workflow = fs.readFileSync(path.join(root, ".github", "workflows", "release.yml"), "utf8");
  const preflight = workflow.indexOf("  npm-registry-preflight:");
  const nativeBuild = workflow.indexOf("  build-native:");

  assert.ok(preflight >= 0, "npm registry preflight job is missing");
  assert.ok(nativeBuild > preflight, "npm registry preflight must precede native builds");
  assert.match(workflow, /needs: \[prepare, nightly-meta, npm-registry-preflight\]/);
  assert.match(workflow, /needs\.npm-registry-preflight\.result == 'success'/);
});

test("repositorySlug normalizes npm repository formats", () => {
  assert.equal(
    repositorySlug({ url: "git+https://github.com/jasonsuhari/gridbash.git" }),
    "jasonsuhari/gridbash",
  );
  assert.equal(repositorySlug("git@github.com:jasonsuhari/gridbash.git"), "jasonsuhari/gridbash");
  assert.equal(repositorySlug("github:jasonsuhari/gridbash"), "jasonsuhari/gridbash");
});

test("parseOwners extracts npm usernames", () => {
  assert.deepEqual(
    parseOwners(
      "jasonmatthewsuhari <jasonmatthewsuhari@gmail.com>\nrelease-bot <bot@example.com>",
    ),
    ["jasonmatthewsuhari", "release-bot"],
  );
});

test("validatePackage accepts the expected owner and repository", () => {
  assert.deepEqual(
    validatePackage({
      expectedName: "gridbash-win32-x64",
      expectedOwner: "jasonmatthewsuhari",
      expectedRepository: "jasonsuhari/gridbash",
      metadata: {
        name: "gridbash-win32-x64",
        version: "0.2.0",
        repository: { url: "git+https://github.com/jasonsuhari/gridbash.git" },
      },
      owners: ["jasonmatthewsuhari"],
    }),
    { errors: [], reclaimedSecurityPlaceholder: false },
  );
});

test("validatePackage permits a transferred npm security placeholder", () => {
  assert.deepEqual(
    validatePackage({
      expectedName: "gridbash-win32-x64",
      expectedOwner: "jasonmatthewsuhari",
      expectedRepository: "jasonsuhari/gridbash",
      metadata: {
        name: "gridbash-win32-x64",
        version: "0.0.1-security",
        repository: "npm/security-holder",
      },
      owners: ["jasonmatthewsuhari"],
    }),
    { errors: [], reclaimedSecurityPlaceholder: true },
  );
});

test("validatePackage reports ownership and repository drift", () => {
  const result = validatePackage({
    expectedName: "gridbash-win32-x64",
    expectedOwner: "jasonmatthewsuhari",
    expectedRepository: "jasonsuhari/gridbash",
    metadata: {
      name: "gridbash-win32-x64",
      version: "0.2.0",
      repository: "someone-else/gridbash",
    },
    owners: ["someone-else"],
  });

  assert.equal(result.reclaimedSecurityPlaceholder, false);
  assert.match(result.errors.join("\n"), /expected owner jasonmatthewsuhari/);
  assert.match(result.errors.join("\n"), /expected repository jasonsuhari\/gridbash/);
});
