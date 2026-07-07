# Fix terminal R autoinput regression

Date: 2026-07-07
Release target: unreleased

## Summary

- Fixed a terminal startup regression where cursor-position query responses
  could be replayed and leak a stray `R` into child terminals.
- Added a release guard that blocks releases while origin still has unmerged
  task branches, unless the releaser explicitly confirms a branch review.

## What Changed

- The PTY terminal-query scanner now keeps only enough trailing bytes to detect
  split escape sequences. It no longer keeps a complete `ESC [ 6 n` query in
  the scan tail, which prevented duplicate cursor-position responses.
- Added regression tests for split cursor-position queries and for complete
  query replay prevention.
- `npm/scripts/release.js` now refreshes origin branch refs and fails releases
  when unmerged `chore/`, `docs/`, `feat/`, `fix/`, `refactor/`, or `test/`
  branches remain.
- Documented the new release branch queue check in `docs/RELEASING.md`.

## Why It Matters

- The prior fix lived on `origin/fix/prevent-terminal-r-autoinput`, but `v0.1.5`
  was cut from `main` without that branch merged. That let the regression ship
  through the GitHub release artifact.
- Parallel agent work needs an explicit branch queue check before releases, or
  valid fixes can remain stranded on task branches.

## Validation

- Run `cargo fmt --check`.
- Run `cargo test`.
- Run `cargo clippy -- -D warnings`.
- Run `node npm/scripts/release.js --help`.
- Confirm the release guard reports unmerged origin task branches.

## Release Notes

- Fixes stray `R` input caused by repeated terminal cursor-position responses.
- Release tooling now catches unmerged task branches before creating a release.
