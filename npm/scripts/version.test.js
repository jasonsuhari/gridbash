const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  nightlyVersion,
  setProjectVersion,
} = require("./version");

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function projectFixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gridbash-version-"));
  writeJson(path.join(root, "package.json"), {
    name: "gridbash",
    version: "0.1.6",
    optionalDependencies: {
      "gridbash-linux-x64": "0.1.6",
      "gridbash-win32-x64": "0.1.6",
    },
  });
  fs.writeFileSync(
    path.join(root, "Cargo.toml"),
    `[package]\nname = "gridbash"\nversion = "0.1.6"\n\n[dependencies]\nanyhow = "1.0"\n`,
  );
  writeJson(path.join(root, "npm", "platforms", "linux-x64", "package.json"), {
    name: "gridbash-linux-x64",
    version: "0.1.6",
  });
  writeJson(path.join(root, "npm", "platforms", "win32-x64", "package.json"), {
    name: "gridbash-win32-x64",
    version: "0.1.6",
  });
  return root;
}

test("setProjectVersion keeps every package on one exact version", (t) => {
  const root = projectFixture();
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const result = setProjectVersion(root, "0.2.0-nightly.20260713.42.gabcdef123456");
  const rootPackage = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));

  assert.equal(rootPackage.version, "0.2.0-nightly.20260713.42.gabcdef123456");
  assert.deepEqual(rootPackage.optionalDependencies, {
    "gridbash-linux-x64": "0.2.0-nightly.20260713.42.gabcdef123456",
    "gridbash-win32-x64": "0.2.0-nightly.20260713.42.gabcdef123456",
  });
  assert.match(
    fs.readFileSync(path.join(root, "Cargo.toml"), "utf8"),
    /version = "0\.2\.0-nightly\.20260713\.42\.gabcdef123456"/,
  );
  assert.equal(result.nativeManifests.length, 2);
  for (const manifest of result.nativeManifests) {
    assert.equal(
      JSON.parse(fs.readFileSync(manifest, "utf8")).version,
      "0.2.0-nightly.20260713.42.gabcdef123456",
    );
  }
});

test("setProjectVersion validates the complete package contract before writing", (t) => {
  const root = projectFixture();
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const packagePath = path.join(root, "package.json");
  const packageJson = JSON.parse(fs.readFileSync(packagePath, "utf8"));
  delete packageJson.optionalDependencies["gridbash-linux-x64"];
  writeJson(packagePath, packageJson);

  assert.throws(
    () => setProjectVersion(root, "0.2.0"),
    /root optionalDependencies is missing gridbash-linux-x64/,
  );
  assert.equal(JSON.parse(fs.readFileSync(packagePath, "utf8")).version, "0.1.6");
});

test("nightlyVersion reuses a prerelease core and advances a stable patch", () => {
  const sha = "ABCDEF1234567890ABCDEF1234567890ABCDEF12";
  assert.equal(
    nightlyVersion("0.2.0-macos.1", "20260713", "42", sha),
    "0.2.0-nightly.20260713.42.gabcdef123456",
  );
  assert.equal(
    nightlyVersion("0.2.0", "20260714", "43", sha),
    "0.3.0-nightly.20260714.43.gabcdef123456",
  );
});

test("release workflow keeps local tarballs and GitHub fallback available", () => {
  const workflow = fs.readFileSync(
    path.join(__dirname, "..", "..", ".github", "workflows", "release.yml"),
    "utf8",
  );

  assert.match(
    workflow,
    /npm publish "\.\/\$tarball" --access public --provenance --tag "\$dist_tag"/,
  );
  assert.doesNotMatch(workflow, /npm publish "\$tarball"/);
  assert.match(
    workflow,
    /- name: Create or update GitHub release\r?\n\s+if: \$\{\{ !cancelled\(\) \}\}/,
  );
});
