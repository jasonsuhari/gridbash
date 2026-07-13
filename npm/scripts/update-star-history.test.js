const assert = require("node:assert/strict");
const { test } = require("node:test");

const {
  fetchStarData,
  niceStarMaximum,
  normalizeHistory,
  renderChart,
} = require("./update-star-history.js");

function jsonResponse(payload, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

test("normalizeHistory sorts timestamps and accumulates stars by UTC day", () => {
  const points = normalizeHistory(
    "2026-07-01T12:00:00Z",
    [
      { starred_at: "2026-07-03T18:00:00Z" },
      { starred_at: "2026-07-02T10:00:00Z" },
      { starred_at: "2026-07-03T09:00:00Z" },
    ],
    new Date("2026-07-04T20:00:00Z"),
  );

  assert.deepEqual(
    points.map(({ day, stars }) => ({ day, stars })),
    [
      { day: "2026-07-01", stars: 0 },
      { day: "2026-07-02", stars: 1 },
      { day: "2026-07-03", stars: 3 },
      { day: "2026-07-04", stars: 3 },
    ],
  );
});

test("normalizeHistory safely handles a repository and stars from the same day", () => {
  const points = normalizeHistory(
    "2026-07-04T01:00:00Z",
    [{ starred_at: "2026-07-04T02:00:00Z" }],
    new Date("2026-07-04T20:00:00Z"),
  );
  assert.deepEqual(points.map(({ day, stars }) => ({ day, stars })), [
    { day: "2026-07-04", stars: 1 },
  ]);
});

test("niceStarMaximum leaves chart headroom at small and larger totals", () => {
  assert.equal(niceStarMaximum(0), 5);
  assert.equal(niceStarMaximum(4), 5);
  assert.equal(niceStarMaximum(41), 50);
  assert.equal(niceStarMaximum(132), 150);
});

test("fetchStarData requests timestamp media and paginates stargazers", async () => {
  const calls = [];
  const fetchImpl = async (url, options) => {
    calls.push({ url, accept: options.headers.Accept });
    if (url === "https://api.example/repos/owner/repo") {
      return jsonResponse({ created_at: "2026-07-01T00:00:00Z" });
    }
    const page = new URL(url).searchParams.get("page");
    return jsonResponse(
      page === "1"
        ? Array.from({ length: 100 }, (_, index) => ({
            starred_at: `2026-07-${String((index % 9) + 1).padStart(2, "0")}T00:00:00Z`,
          }))
        : [{ starred_at: "2026-07-10T00:00:00Z" }],
    );
  };

  const data = await fetchStarData(fetchImpl, "https://api.example", "owner/repo", "token");
  assert.equal(data.stargazers.length, 101);
  assert.equal(calls.length, 3);
  assert.equal(calls[1].accept, "application/vnd.github.star+json");
  assert.match(calls[2].url, /page=2/);
});

test("renderChart produces accessible responsive-theme SVG without invalid numbers", () => {
  const points = normalizeHistory(
    "2026-07-01T00:00:00Z",
    [{ starred_at: "2026-07-02T00:00:00Z" }],
    new Date("2026-07-03T00:00:00Z"),
  );
  const svg = renderChart("owner/<repo>", points, new Date("2026-07-03T00:00:00Z"));

  assert.match(svg, /role="img"/);
  assert.match(svg, /prefers-color-scheme: dark/);
  assert.match(svg, /owner\/&lt;repo&gt;/);
  assert.match(svg, /Current total: 1 star\./);
  assert.doesNotMatch(svg, /NaN|Infinity/);
});
