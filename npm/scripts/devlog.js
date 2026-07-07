const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..", "..");
const args = process.argv.slice(2);

function fail(message) {
  console.error(`devlog: ${message}`);
  process.exit(1);
}

function readFlag(name) {
  const index = args.indexOf(name);
  if (index === -1) {
    return undefined;
  }
  return args[index + 1];
}

function today() {
  return new Date().toISOString().slice(0, 10);
}

function slugify(value) {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

const title = readFlag("--title") || args.find((arg) => !arg.startsWith("--"));
if (!title) {
  fail('pass a title, for example: npm run devlog -- --title "Startup grid picker"');
}

const date = readFlag("--date") || today();
const slug = slugify(readFlag("--slug") || title);
if (!slug) {
  fail("title did not produce a usable slug");
}

const outDir = path.join(root, "docs", "devlogs");
const outPath = path.join(outDir, `${date}-${slug}.md`);
if (fs.existsSync(outPath)) {
  fail(`already exists: ${path.relative(root, outPath)}`);
}

fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(
  outPath,
  `# ${title}

Date: ${date}
Release target: unreleased

## Summary

- 

## What Changed

- 

## Why It Matters

- 

## Validation

- 

## Release Notes

- 
`,
);

console.log(path.relative(root, outPath));
