const fs = require("node:fs");

const MAX_ISSUE_TEXT = 50_000;
const TYPE_LABELS = new Set([
  "bug",
  "documentation",
  "enhancement",
  "question",
  "type:design",
  "type:maintenance",
  "type:test",
]);

const PREFIX_LABELS = new Map([
  ["bug", "bug"],
  ["build", "type:maintenance"],
  ["chore", "type:maintenance"],
  ["ci", "type:maintenance"],
  ["design", "type:design"],
  ["docs", "documentation"],
  ["feat", "enhancement"],
  ["fix", "bug"],
  ["perf", "type:maintenance"],
  ["question", "question"],
  ["refactor", "type:maintenance"],
  ["release", "type:maintenance"],
  ["rfc", "type:design"],
  ["security", "bug"],
  ["test", "type:test"],
]);

const FORM_AREA_LABELS = new Map([
  ["documentation", ["area:docs"]],
  ["npm packaging", ["area:packaging"]],
  ["profiles/configuration", ["area:profiles", "area:config"]],
  ["pty/process behavior", ["area:pty"]],
  ["tui controls", ["area:tui"]],
]);

const AREA_RULES = [
  {
    label: "area:pty",
    titlePattern: /\b(?:conpty|pty|pseudo[- ]terminal|process lifecycle|spawn(?:ing|ed)?|stdin|stdout|stderr|terminal i\/o)\b/i,
  },
  {
    label: "area:tui",
    titlePattern: /\b(?:tui|ratatui|terminal ui|pane settings|overlay|modal|focus|keybindings?|keyboard|mouse|shortcuts?|render(?:ing)?|layout|tabs?|scroll(?:ing)?|(?:pane|tui) controls?|alt\+[a-z0-9]|command output)\b/i,
  },
  {
    label: "area:profiles",
    titlePattern: /\b(?:terminal profiles?|agent profiles?|profile detection|shell profiles?|shell detection|codex|claude|gemini|authentication|auth usage)\b/i,
  },
  {
    label: "area:composer",
    titlePattern: /\b(?:composer|orchestrat(?:e|es|ed|ing|ion)|manager goals?|saved setups?)\b/i,
  },
  {
    label: "area:config",
    titlePattern: /\b(?:config(?:uration)?|schema|toml|preferences?|persist(?:ed|ence|ing)?|environment variables?)\b/i,
  },
  {
    label: "area:packaging",
    titlePattern: /\b(?:npm|packages?|packaging|tarball|install(?:er|ation|ing)?|publish(?:ing)?|release|signing|notarization|binar(?:y|ies)|distribution|dependenc(?:y|ies)|arm64|x64|github actions?|workflows?|ci)\b/i,
  },
  {
    label: "area:docs",
    titlePattern: /\b(?:documentation|docs?|readme|devlogs?|website|command reference|guides?)\b/i,
  },
  {
    label: "area:architecture",
    titlePattern: /\b(?:architecture|daemon|protocol|agent api|security|credentials?|permissions?|refactor|monolith|module (?:boundary|ownership))\b/i,
  },
];

const WINDOWS_PATTERN = /\b(?:windows|win32|powershell|conpty)\b/i;

function existingLabelNames(issue) {
  return new Set(
    (issue.labels || [])
      .map((label) => (typeof label === "string" ? label : label && label.name))
      .filter(Boolean),
  );
}

function issueText(issue) {
  const body = String(issue.body || "").replace(/^#{1,6}\s+.*$/gm, "\n");
  return (String(issue.title || "") + "\n" + body).slice(0, MAX_ISSUE_TEXT);
}

function extractAreaAnswer(body) {
  const match = String(body || "").match(
    /(?:^|\r?\n)###\s+Area\s*\r?\n+([\s\S]*?)(?=\r?\n###\s+|$)/i,
  );
  if (!match) {
    return undefined;
  }
  return match[1].trim().split(/\r?\n/, 1)[0].trim().toLowerCase();
}

function typeLabelForTitle(title) {
  const text = String(title || "").trim();
  const conventional = text.match(/^([a-z]+)(?:\([^)]+\))?!?:\s+/i);
  if (conventional) {
    return PREFIX_LABELS.get(conventional[1].toLowerCase());
  }

  if (/^(?:add|allow|create|enable|improve|introduce|make|provide|show|support)\b/i.test(text)) {
    return "enhancement";
  }
  if (/^(?:avoid|correct|fix|prevent|resolve|restore|stop)\b/i.test(text)) {
    return "bug";
  }
  if (/^(?:can|does|how|is|what|why)\b.*\?$/i.test(text)) {
    return "question";
  }
  return undefined;
}

function labelsForIssue(issue) {
  const existing = existingLabelNames(issue);
  const labels = [];
  const add = (label) => {
    if (label && !existing.has(label) && !labels.includes(label)) {
      labels.push(label);
    }
  };

  let typeLabel = [...existing].find((label) => TYPE_LABELS.has(label));
  if (!typeLabel) {
    typeLabel = typeLabelForTitle(issue.title);
    add(typeLabel);
  }

  if (
    issue.state !== "closed" &&
    ![...existing].some((label) => label.startsWith("status:"))
  ) {
    add("status:needs-triage");
  }

  if (![...existing].some((label) => label.startsWith("area:"))) {
    const areaAnswer = extractAreaAnswer(issue.body);
    const formLabels = FORM_AREA_LABELS.get(areaAnswer);
    if (formLabels) {
      formLabels.forEach(add);
    } else if (typeLabel === "documentation") {
      add("area:docs");
    } else if (areaAnswer !== "other") {
      const title = String(issue.title || "").slice(0, MAX_ISSUE_TEXT);
      AREA_RULES.filter((rule) => rule.titlePattern.test(title)).forEach((rule) => add(rule.label));
    }
  }

  if (
    ![...existing].some((label) => label.startsWith("platform:")) &&
    WINDOWS_PATTERN.test(issueText(issue))
  ) {
    add("platform:windows");
  }

  return labels;
}

function requiredEnvironment(name, environment) {
  const value = environment[name];
  if (!value) {
    throw new Error("missing required environment variable " + name);
  }
  return value;
}

function repositoryName(value) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value)) {
    throw new Error("invalid GITHUB_REPOSITORY value");
  }
  return value;
}

function issueNumber(value) {
  const text = String(value || "").trim();
  if (!/^[1-9]\d*$/.test(text)) {
    throw new Error("issue number must be a positive integer");
  }
  const parsed = Number(text);
  if (!Number.isSafeInteger(parsed) || parsed < 1) {
    throw new Error("issue number must be a positive integer");
  }
  return parsed;
}

async function requestJson(fetchImpl, url, options) {
  const response = await fetchImpl(url, {
    method: options.method || "GET",
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: "Bearer " + options.token,
      "Content-Type": "application/json",
      "User-Agent": "gridbash-issue-labeler",
      "X-GitHub-Api-Version": "2022-11-28",
    },
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });
  const text = await response.text();
  let payload;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = text;
    }
  }
  if (!response.ok) {
    const message = typeof payload === "object" && payload && payload.message
      ? payload.message
      : text || "unknown error";
    throw new Error((options.method || "GET") + " " + new URL(url).pathname + " failed with " + response.status + ": " + message);
  }
  return payload;
}

async function fetchIssue(fetchImpl, apiBase, repository, number, token) {
  return requestJson(
    fetchImpl,
    apiBase + "/repos/" + repository + "/issues/" + number,
    { token },
  );
}

async function addIssueLabels(fetchImpl, apiBase, repository, number, labels, token) {
  return requestJson(
    fetchImpl,
    apiBase + "/repos/" + repository + "/issues/" + number + "/labels",
    { method: "POST", token, body: { labels } },
  );
}

async function runIssueLabeler(options = {}) {
  const environment = options.environment || process.env;
  const fetchImpl = options.fetchImpl || fetch;
  const logger = options.logger || console;
  const repository = repositoryName(requiredEnvironment("GITHUB_REPOSITORY", environment));
  const token = requiredEnvironment("GITHUB_TOKEN", environment);
  const apiBase = (environment.GITHUB_API_URL || "https://api.github.com").replace(/\/+$/, "");
  const payload = options.payload || JSON.parse(
    fs.readFileSync(requiredEnvironment("GITHUB_EVENT_PATH", environment), "utf8"),
  );

  let issue = payload.issue;
  if (!issue) {
    const number = issueNumber(environment.ISSUE_NUMBER || (payload.inputs && payload.inputs.issue_number));
    issue = await fetchIssue(fetchImpl, apiBase, repository, number, token);
  }
  if (issue.pull_request) {
    throw new Error("#" + issue.number + " is a pull request, not an issue");
  }

  const labels = labelsForIssue(issue);
  if (labels.length === 0) {
    logger.log("issue-labeler: #" + issue.number + " already has all matching labels");
    return [];
  }

  await addIssueLabels(fetchImpl, apiBase, repository, issueNumber(issue.number), labels, token);
  logger.log("issue-labeler: added " + labels.join(", ") + " to #" + issue.number);
  return labels;
}

if (require.main === module) {
  runIssueLabeler().catch((error) => {
    console.error("issue-labeler: " + error.message);
    process.exitCode = 1;
  });
}

module.exports = {
  addIssueLabels,
  extractAreaAnswer,
  fetchIssue,
  labelsForIssue,
  runIssueLabeler,
  typeLabelForTitle,
};
