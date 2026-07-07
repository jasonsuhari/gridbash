# Expose latest version and devlogs

Date: 2026-07-07
Release target: unreleased

## Summary

- Add quick latest-version signals to the README/npm page.
- Include devlog and release-note docs in published npm packages.
- Track this request through GitHub issue #13.

## What Changed

- Added npm and GitHub release badges to the README.
- Added a `Release Status & Devlogs` README section.
- Added `docs/devlogs/`, `docs/releases/`, and `docs/RELEASING.md` to `package.json` package files.

## Why It Matters

- Readers can quickly tell what version is live on npm or GitHub.
- npm package users can inspect the same logs that are available in the repo.
- Future release notes have a predictable public place to live.

## Validation

- Verified current live state with `gh release list`, remote tags, and `npm view gridbash`.
- Validated package contents with `npm pack --dry-run --ignore-scripts`.

## Release Notes

- README now exposes npm and GitHub release status badges.
- Published npm packages now include devlogs and release notes.
