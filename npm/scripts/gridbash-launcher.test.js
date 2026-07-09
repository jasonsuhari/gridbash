const assert = require("node:assert/strict");
const http = require("node:http");
const { test } = require("node:test");

const {
  checkForUpdate,
  compareVersions,
  formatUpdateNotice,
  shouldSkipUpdateCheck,
  updateCheckTimeoutMs,
} = require("../bin/gridbash.js");

function serveJson(payload) {
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "Content-Type": "application/json" });
    response.end(JSON.stringify(payload));
  });

  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      resolve({
        url: `http://127.0.0.1:${port}/latest`,
        close: () => new Promise((done) => server.close(done)),
      });
    });
  });
}

test("compareVersions handles newer, older, and prerelease versions", () => {
  assert.equal(compareVersions("0.1.6", "0.1.5"), 1);
  assert.equal(compareVersions("v0.1.5", "0.1.5"), 0);
  assert.equal(compareVersions("0.1.4", "0.1.5"), -1);
  assert.equal(compareVersions("0.2.0", "0.1.99"), 1);
  assert.equal(compareVersions("1.0.0", "1.0.0-beta.1"), 1);
});

test("shouldSkipUpdateCheck preserves help, version, MCP, and non-TTY paths", () => {
  assert.equal(shouldSkipUpdateCheck(["--version"], {}, { isTTY: true }), true);
  assert.equal(shouldSkipUpdateCheck(["--mcp"], {}, { isTTY: true }), true);
  assert.equal(shouldSkipUpdateCheck([], { GRIDBASH_NO_UPDATE_CHECK: "1" }, { isTTY: true }), true);
  assert.equal(shouldSkipUpdateCheck([], {}, { isTTY: false }), true);
  assert.equal(shouldSkipUpdateCheck(["2x2"], {}, { isTTY: true }), false);
});

test("updateCheckTimeoutMs clamps overrides", () => {
  assert.equal(updateCheckTimeoutMs({ GRIDBASH_UPDATE_CHECK_TIMEOUT_MS: "25" }), 25);
  assert.equal(updateCheckTimeoutMs({ GRIDBASH_UPDATE_CHECK_TIMEOUT_MS: "10000" }), 5000);
});

test("checkForUpdate returns latest release only when it is newer", async () => {
  const server = await serveJson({
    tag_name: "v0.1.6",
    html_url: "https://github.com/jasonsuhari/gridbash/releases/tag/v0.1.6",
  });

  try {
    const latest = await checkForUpdate("0.1.5", {
      GRIDBASH_UPDATE_CHECK_URL: server.url,
      GRIDBASH_UPDATE_CHECK_TIMEOUT_MS: "500",
    });

    assert.deepEqual(latest, {
      tagName: "v0.1.6",
      version: "0.1.6",
      url: "https://github.com/jasonsuhari/gridbash/releases/tag/v0.1.6",
    });
    assert.match(formatUpdateNotice("0.1.5", latest), /update available v0\.1\.6/);

    const current = await checkForUpdate("0.1.6", {
      GRIDBASH_UPDATE_CHECK_URL: server.url,
      GRIDBASH_UPDATE_CHECK_TIMEOUT_MS: "500",
    });
    assert.equal(current, undefined);
  } finally {
    await server.close();
  }
});
