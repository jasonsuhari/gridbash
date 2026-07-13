# Automatically refreshed star history chart

Date: 2026-07-13
Release target: unreleased

## Summary

- Add a live README chart showing GridBash's cumulative GitHub star history.

## What Changed

- Added a dependency-free SVG generator backed by GitHub's timestamped
  stargazer API.
- Added a pinned daily/manual workflow that refreshes the chart with the
  repository's short-lived token and commits only changed output.
- Added accessible light/dark chart styling, deterministic tests, and npm
  package inclusion for the generated asset.

## Why It Matters

- Visitors can see project growth directly in the README without giving a
  third-party service a persistent or encrypted repository token.

## Validation

- `npm run test:star-history`
- Live GitHub API generation against `jasonsuhari/gridbash`.
- `actionlint` for the refresh workflow.
- `npm pack --dry-run --ignore-scripts`.
- Live workflow dispatch after merge.

## Release Notes

- The README now includes an automatically refreshed GitHub star history chart.
