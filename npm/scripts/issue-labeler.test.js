const assert = require("node:assert/strict");
const { test } = require("node:test");

const {
  extractAreaAnswer,
  labelsForIssue,
  runIssueLabeler,
  typeLabelForTitle,
} = require("./issue-labeler.js");

function jsonResponse(payload, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

test("typeLabelForTitle classifies conventional and conservative titles", () => {
  const cases = new Map([
    ["fix: stop a crash", "bug"],
    ["feat(tui): add tabs", "enhancement"],
    ["docs!: replace the guide", "documentation"],
    ["test: cover overlays", "type:test"],
    ["design: define the protocol", "type:design"],
    ["refactor(core): split modules", "type:maintenance"],
    ["security: scope credentials", "bug"],
    ["Make manager goals orchestrate panes", "enhancement"],
    ["Prevent startup failures", "bug"],
    ["How does profile detection work?", "question"],
  ]);

  for (const [title, expected] of cases) {
    assert.equal(typeLabelForTitle(title), expected, title);
  }
  assert.equal(typeLabelForTitle("Investigate pane behavior"), undefined);
});

test("extractAreaAnswer reads the structured feature form response", () => {
  const body = "### Problem\n\nHard to use.\n\n### Area\n\nProfiles/configuration\n\n### Checklist\n\n- [x] Done";
  assert.equal(extractAreaAnswer(body), "profiles/configuration");
});

test("labelsForIssue maps form area answers without duplicating form labels", () => {
  const labels = labelsForIssue({
    title: "feat: improve profile setup",
    body: "### Area\n\nProfiles/configuration",
    labels: [{ name: "enhancement" }],
  });

  assert.deepEqual(labels, [
    "status:needs-triage",
    "area:profiles",
    "area:config",
  ]);
});

test("labelsForIssue classifies CLI-created issues from strong title signals", () => {
  const labels = labelsForIssue({
    title: "fix: isolate Codex SQLite state per pane",
    body: "Shell profiles need a unique environment variable while preserving shared config.",
    labels: [],
  });

  assert.deepEqual(labels, [
    "bug",
    "status:needs-triage",
    "area:profiles",
  ]);
});

test("labelsForIssue ignores incidental areas mentioned only in the body", () => {
  const labels = labelsForIssue({
    title: "feat: add automatic PR review agent",
    body: [
      "## Problem",
      "",
      "Reviews should cover PTY behavior and config persistence.",
      "",
      "## Requested behavior",
      "",
      "Run a trusted workflow.",
      "",
      "## Acceptance checks",
      "",
      "- Update the docs.",
    ].join("\n"),
    labels: [],
  });

  assert.deepEqual(labels, [
    "enhancement",
    "status:needs-triage",
  ]);
});

test("agent-control security issues are architecture, not TUI controls", () => {
  const labels = labelsForIssue({
    title: "security: scope agent-control credentials and permissions per pane",
    body: "",
    labels: [],
  });

  assert.deepEqual(labels, [
    "bug",
    "status:needs-triage",
    "area:architecture",
  ]);
});

test("labelsForIssue ignores bug-form metadata when inferring areas", () => {
  const labels = labelsForIssue({
    title: "bug: overlay is cut off",
    body: [
      "### GridBash version or commit",
      "",
      "0.1.6",
      "",
      "### Windows version",
      "",
      "Windows 11",
      "",
      "### Host terminal",
      "",
      "PowerShell",
      "",
      "### Launch command",
      "",
      "gridbash 2x2 --profile powershell",
      "",
      "### Steps to reproduce",
      "",
      "Open the help overlay.",
      "",
      "### Expected behavior",
      "",
      "The overlay fits the layout.",
      "",
      "### Actual behavior",
      "",
      "The overlay is clipped.",
      "",
      "### Checklist",
      "",
      "- [x] I searched existing issues.",
    ].join("\n"),
    labels: [{ name: "bug" }],
  });

  assert.deepEqual(labels, [
    "status:needs-triage",
    "area:tui",
    "platform:windows",
  ]);
});

test("a cross-platform bug form heading does not imply Windows", () => {
  const labels = labelsForIssue({
    title: "bug: startup exits unexpectedly",
    body: [
      "### GridBash version or commit",
      "",
      "0.1.6",
      "",
      "### Windows version",
      "",
      "N/A - Ubuntu 24.04",
      "",
      "### Host terminal",
      "",
      "bash",
      "",
      "### Launch command",
      "",
      "gridbash 2x2",
    ].join("\n"),
    labels: [{ name: "bug" }],
  });

  assert.deepEqual(labels, ["status:needs-triage"]);
});

test("documentation issues do not infer product areas from prose", () => {
  const labels = labelsForIssue({
    title: "docs: improve the README layout",
    body: "Update the installation guide and package examples.",
    labels: [],
  });

  assert.deepEqual(labels, [
    "documentation",
    "status:needs-triage",
    "area:docs",
  ]);
});

test("labelsForIssue detects packaging and Windows without incidental docs", () => {
  const labels = labelsForIssue({
    title: "chore: test npm installer workflow",
    body: "Validate PowerShell package installation docs and CI.",
    labels: [],
  });

  assert.deepEqual(labels, [
    "type:maintenance",
    "status:needs-triage",
    "area:packaging",
    "platform:windows",
  ]);
});

test("labelsForIssue leaves priority and resolution decisions to maintainers", () => {
  const labels = labelsForIssue({
    title: "Investigate an unusual report",
    body: "No strong classification signal.",
    labels: [],
  });

  assert.deepEqual(labels, ["status:needs-triage"]);
  assert.equal(labels.some((label) => label.startsWith("priority:")), false);
});

test("closed issues do not receive a needs-triage status during backfill", () => {
  const labels = labelsForIssue({
    title: "docs: archive an old guide",
    body: "",
    labels: [],
    state: "closed",
  });

  assert.deepEqual(labels, ["documentation", "area:docs"]);
});

test("labelsForIssue preserves existing category, status, area, and platform labels", () => {
  const labels = labelsForIssue({
    title: "fix: change a Windows TUI layout",
    body: "PowerShell overlay rendering.",
    labels: [
      { name: "enhancement" },
      { name: "status:accepted" },
      { name: "area:architecture" },
      { name: "platform:windows" },
    ],
  });

  assert.deepEqual(labels, []);
});

test("issue text is handled only as inert classifier input", () => {
  const labels = labelsForIssue({
    title: "Investigate report",
    body: "$" + "{{ secrets.GITHUB_TOKEN }}; process.exit(1)",
    labels: [],
  });

  assert.deepEqual(labels, ["status:needs-triage"]);
});

test("runIssueLabeler posts only missing labels from an issue event", async () => {
  const requests = [];
  const logs = [];
  const fetchImpl = async (url, options) => {
    requests.push({ url, options });
    return jsonResponse([{ name: "bug" }, { name: "status:needs-triage" }]);
  };

  const labels = await runIssueLabeler({
    environment: {
      GITHUB_API_URL: "https://api.example",
      GITHUB_REPOSITORY: "owner/repo",
      GITHUB_TOKEN: "token",
    },
    payload: {
      issue: { number: 42, title: "fix: crash on Windows", body: "", labels: [] },
    },
    fetchImpl,
    logger: { log: (message) => logs.push(message) },
  });

  assert.deepEqual(labels, ["bug", "status:needs-triage", "platform:windows"]);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, "https://api.example/repos/owner/repo/issues/42/labels");
  assert.equal(requests[0].options.method, "POST");
  assert.deepEqual(JSON.parse(requests[0].options.body), { labels });
  assert.match(logs[0], /added bug, status:needs-triage, platform:windows to #42/);
});

test("runIssueLabeler can classify an existing issue from manual dispatch", async () => {
  const requests = [];
  const fetchImpl = async (url, options) => {
    requests.push({ url, options });
    if (options.method === "GET") {
      return jsonResponse({
        number: 99,
        title: "feat: add manager goal orchestration",
        body: "",
        labels: [],
      });
    }
    return jsonResponse([]);
  };

  const labels = await runIssueLabeler({
    environment: {
      GITHUB_API_URL: "https://api.example/",
      GITHUB_REPOSITORY: "owner/repo",
      GITHUB_TOKEN: "token",
      ISSUE_NUMBER: "99",
    },
    payload: { inputs: { issue_number: "99" } },
    fetchImpl,
    logger: { log() {} },
  });

  assert.deepEqual(labels, [
    "enhancement",
    "status:needs-triage",
    "area:composer",
  ]);
  assert.equal(requests.length, 2);
  assert.equal(requests[0].url, "https://api.example/repos/owner/repo/issues/99");
  assert.equal(requests[1].url, "https://api.example/repos/owner/repo/issues/99/labels");
});

test("runIssueLabeler skips the API when every matching label exists", async () => {
  let called = false;
  const labels = await runIssueLabeler({
    environment: {
      GITHUB_REPOSITORY: "owner/repo",
      GITHUB_TOKEN: "token",
    },
    payload: {
      issue: {
        number: 7,
        title: "feat: add tabs",
        body: "",
        labels: ["enhancement", "status:accepted", "area:tui"],
      },
    },
    fetchImpl: async () => {
      called = true;
      return jsonResponse([]);
    },
    logger: { log() {} },
  });

  assert.deepEqual(labels, []);
  assert.equal(called, false);
});

test("runIssueLabeler refuses to label pull requests during manual dispatch", async () => {
  let calls = 0;
  const fetchImpl = async () => {
    calls += 1;
    return jsonResponse({
      number: 12,
      title: "feat: pull request",
      body: "",
      labels: [],
      pull_request: { url: "https://api.example/pulls/12" },
    });
  };

  await assert.rejects(
    runIssueLabeler({
      environment: {
        GITHUB_REPOSITORY: "owner/repo",
        GITHUB_TOKEN: "token",
        ISSUE_NUMBER: "12",
      },
      payload: { inputs: { issue_number: "12" } },
      fetchImpl,
      logger: { log() {} },
    }),
    /is a pull request, not an issue/,
  );
  assert.equal(calls, 1);
});

test("runIssueLabeler rejects malformed manual issue numbers", async () => {
  for (const invalid of ["0", "12oops", "12.5", "12e3"]) {
    let called = false;
    await assert.rejects(
      runIssueLabeler({
        environment: {
          GITHUB_REPOSITORY: "owner/repo",
          GITHUB_TOKEN: "token",
          ISSUE_NUMBER: invalid,
        },
        payload: { inputs: { issue_number: invalid } },
        fetchImpl: async () => {
          called = true;
          return jsonResponse({});
        },
        logger: { log() {} },
      }),
      /issue number must be a positive integer/,
    );
    assert.equal(called, false);
  }
});
