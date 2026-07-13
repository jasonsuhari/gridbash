const fs = require("node:fs");
const path = require("node:path");

const WIDTH = 960;
const HEIGHT = 480;
const PADDING = { top: 78, right: 42, bottom: 68, left: 72 };

function requiredEnvironment(name, environment = process.env) {
  const value = environment[name];
  if (!value) {
    throw new Error(`missing required environment variable ${name}`);
  }
  return value;
}

async function requestJson(fetchImpl, url, token, accept = "application/vnd.github+json") {
  const response = await fetchImpl(url, {
    headers: {
      Accept: accept,
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2026-03-10",
    },
  });
  const text = await response.text();
  let payload;
  try {
    payload = text ? JSON.parse(text) : undefined;
  } catch {
    payload = text;
  }
  if (!response.ok) {
    const message = typeof payload === "object" && payload?.message ? payload.message : text;
    throw new Error(`GET ${new URL(url).pathname} failed with ${response.status}: ${message}`);
  }
  return payload;
}

async function fetchStarData(fetchImpl, apiBase, repository, token) {
  const repositoryData = await requestJson(
    fetchImpl,
    `${apiBase}/repos/${repository}`,
    token,
  );
  const stargazers = [];
  for (let page = 1; page <= 1_000; page += 1) {
    const batch = await requestJson(
      fetchImpl,
      `${apiBase}/repos/${repository}/stargazers?per_page=100&page=${page}`,
      token,
      "application/vnd.github.star+json",
    );
    if (!Array.isArray(batch)) {
      throw new Error("stargazers response was not an array");
    }
    stargazers.push(...batch);
    if (batch.length < 100) {
      return { repository: repositoryData, stargazers };
    }
  }
  throw new Error("repository exceeds the 100,000-star chart limit");
}

function utcDay(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    throw new Error(`invalid date: ${value}`);
  }
  return date.toISOString().slice(0, 10);
}

function normalizeHistory(createdAt, stargazers, now = new Date()) {
  const createdDay = utcDay(createdAt);
  const today = utcDay(now);
  const datedStars = stargazers
    .map((entry) => entry.starred_at)
    .filter(Boolean)
    .map(utcDay)
    .sort();
  const pointsByDay = new Map([[createdDay, 0]]);
  datedStars.forEach((day, index) => pointsByDay.set(day, index + 1));
  pointsByDay.set(today, datedStars.length);

  return [...pointsByDay.entries()]
    .map(([day, stars]) => ({ day, time: Date.parse(`${day}T00:00:00Z`), stars }))
    .sort((left, right) => left.time - right.time);
}

function niceStarMaximum(stars) {
  if (stars <= 5) {
    return 5;
  }
  const magnitude = 10 ** Math.floor(Math.log10(stars));
  const interval = magnitude >= stars / 2 ? magnitude / 2 : magnitude;
  return Math.ceil(stars / interval) * interval;
}

function xmlEscape(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

function formatDate(time) {
  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  }).format(new Date(time));
}

function renderChart(repository, points, generatedAt = new Date()) {
  if (!points.length) {
    throw new Error("star history requires at least one point");
  }
  const plotWidth = WIDTH - PADDING.left - PADDING.right;
  const plotHeight = HEIGHT - PADDING.top - PADDING.bottom;
  const firstTime = points[0].time;
  const lastTime = points.at(-1).time;
  const timeSpan = Math.max(1, lastTime - firstTime);
  const currentStars = points.at(-1).stars;
  const starLabel = `${currentStars} star${currentStars === 1 ? "" : "s"}`;
  const starMaximum = niceStarMaximum(currentStars);
  const x = (time) => PADDING.left + ((time - firstTime) / timeSpan) * plotWidth;
  const y = (stars) => PADDING.top + plotHeight - (stars / starMaximum) * plotHeight;
  const linePoints = points.map((point) => `${x(point.time).toFixed(2)},${y(point.stars).toFixed(2)}`);
  if (linePoints.length === 1) {
    linePoints.push(`${(PADDING.left + plotWidth).toFixed(2)},${y(points[0].stars).toFixed(2)}`);
  }
  const linePath = `M ${linePoints.join(" L ")}`;
  const areaPath = `${linePath} L ${(PADDING.left + plotWidth).toFixed(2)},${(
    PADDING.top + plotHeight
  ).toFixed(2)} L ${PADDING.left},${(PADDING.top + plotHeight).toFixed(2)} Z`;

  const horizontalGrid = Array.from({ length: 6 }, (_, index) => {
    const value = (starMaximum * index) / 5;
    const position = y(value);
    return [
      `<line class="grid" x1="${PADDING.left}" y1="${position.toFixed(2)}" x2="${
        WIDTH - PADDING.right
      }" y2="${position.toFixed(2)}" />`,
      `<text class="axis-label" x="${PADDING.left - 14}" y="${(
        position + 5
      ).toFixed(2)}" text-anchor="end">${Math.round(value)}</text>`,
    ].join("\n");
  }).join("\n");

  const dateTicks = Array.from({ length: 5 }, (_, index) => {
    const ratio = index / 4;
    const time = firstTime + timeSpan * ratio;
    const position = PADDING.left + plotWidth * ratio;
    return [
      `<line class="grid vertical" x1="${position.toFixed(2)}" y1="${PADDING.top}" x2="${
        position.toFixed(2)
      }" y2="${PADDING.top + plotHeight}" />`,
      `<text class="axis-label" x="${position.toFixed(2)}" y="${
        HEIGHT - PADDING.bottom + 30
      }" text-anchor="middle">${xmlEscape(formatDate(time))}</text>`,
    ].join("\n");
  }).join("\n");

  const safeRepository = xmlEscape(repository);
  const generatedDay = xmlEscape(utcDay(generatedAt));
  const finalX = x(lastTime).toFixed(2);
  const finalY = y(currentStars).toFixed(2);

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${WIDTH}" height="${HEIGHT}" viewBox="0 0 ${WIDTH} ${HEIGHT}" role="img" aria-labelledby="chart-title chart-description">
  <title id="chart-title">${safeRepository} GitHub star history</title>
  <desc id="chart-description">Cumulative GitHub stars from ${xmlEscape(
    formatDate(firstTime),
  )} through ${xmlEscape(formatDate(lastTime))}. Current total: ${starLabel}.</desc>
  <defs>
    <linearGradient id="area-gradient" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#3b82f6" stop-opacity="0.38" />
      <stop offset="100%" stop-color="#3b82f6" stop-opacity="0.03" />
    </linearGradient>
    <filter id="glow" x="-50%" y="-50%" width="200%" height="200%">
      <feGaussianBlur stdDeviation="3" result="blur" />
      <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
    </filter>
  </defs>
  <style>
    .background { fill: #ffffff; }
    .title { fill: #111827; font: 700 24px ui-sans-serif, system-ui, sans-serif; }
    .subtitle { fill: #6b7280; font: 14px ui-sans-serif, system-ui, sans-serif; }
    .axis-label { fill: #6b7280; font: 12px ui-monospace, SFMono-Regular, Consolas, monospace; }
    .grid { stroke: #e5e7eb; stroke-width: 1; }
    .vertical { stroke-dasharray: 3 6; }
    .line { fill: none; stroke: #2563eb; stroke-width: 4; stroke-linecap: round; stroke-linejoin: round; }
    .point { fill: #2563eb; stroke: #ffffff; stroke-width: 3; }
    .count { fill: #2563eb; font: 700 16px ui-sans-serif, system-ui, sans-serif; }
    @media (prefers-color-scheme: dark) {
      .background { fill: #0d1117; }
      .title { fill: #f0f6fc; }
      .subtitle, .axis-label { fill: #8b949e; }
      .grid { stroke: #30363d; }
      .line { stroke: #58a6ff; }
      .point { fill: #58a6ff; stroke: #0d1117; }
      .count { fill: #58a6ff; }
    }
  </style>
  <rect class="background" width="${WIDTH}" height="${HEIGHT}" rx="18" />
  <text class="title" x="${PADDING.left}" y="38">Star history</text>
  <text class="subtitle" x="${PADDING.left}" y="61">${safeRepository} · updated ${generatedDay}</text>
  ${horizontalGrid}
  ${dateTicks}
  <path d="${areaPath}" fill="url(#area-gradient)" />
  <path class="line" d="${linePath}" />
  <circle class="point" cx="${finalX}" cy="${finalY}" r="6" filter="url(#glow)" />
  <text class="count" x="${Math.max(PADDING.left, Number(finalX) - 12).toFixed(
    2,
  )}" y="${Math.max(PADDING.top + 18, Number(finalY) - 14).toFixed(
    2,
  )}" text-anchor="end">${currentStars} ★</text>
</svg>
`;
}

async function updateStarHistory(options = {}) {
  const environment = options.environment || process.env;
  const fetchImpl = options.fetchImpl || fetch;
  const repository = requiredEnvironment("GITHUB_REPOSITORY", environment);
  const token = requiredEnvironment("GITHUB_TOKEN", environment);
  const apiBase = environment.GITHUB_API_URL || "https://api.github.com";
  const now = new Date(environment.STAR_HISTORY_NOW || Date.now());
  const root = options.root || path.resolve(__dirname, "..", "..");
  const output = path.resolve(
    root,
    environment.STAR_HISTORY_OUTPUT || "docs/assets/gridbash-star-history.svg",
  );
  const data = await fetchStarData(fetchImpl, apiBase, repository, token);
  const points = normalizeHistory(data.repository.created_at, data.stargazers, now);
  const svg = renderChart(repository, points, now);
  const previous = fs.existsSync(output) ? fs.readFileSync(output, "utf8") : undefined;
  if (previous === svg) {
    console.log(`star-history: ${path.relative(root, output)} is current`);
    return false;
  }
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, svg);
  console.log(
    `star-history: wrote ${path.relative(root, output)} with ${data.stargazers.length} stars`,
  );
  return true;
}

if (require.main === module) {
  updateStarHistory().catch((error) => {
    console.error(`star-history: ${error.message}`);
    process.exitCode = 1;
  });
}

module.exports = {
  fetchStarData,
  niceStarMaximum,
  normalizeHistory,
  renderChart,
  updateStarHistory,
  xmlEscape,
};
