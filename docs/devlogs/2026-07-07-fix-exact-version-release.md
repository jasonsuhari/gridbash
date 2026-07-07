# Fix exact version release

Date: 2026-07-07
Release target: unreleased

## Summary

- Fixed exact-version release preparation for versions already present in `Cargo.toml`.
- Tracked the failed CI/CD release attempt through GitHub issue #19.

## What Changed

- `npm/scripts/release.js` now checks that `Cargo.toml` has a version field before replacing it.
- If the requested exact version already matches `Cargo.toml`, the script leaves it unchanged instead of failing.
- Release preparation can still create `docs/releases/vX.Y.Z.md`, commit it, tag it, and push it.

## Why It Matters

- The current merged version is already `0.1.4`, while npm is still at `0.1.0`.
- CI/CD needs to publish the existing `0.1.4` version without forcing an unnecessary `0.1.5` bump.
- Failed exact-version releases can be fixed and rerun cleanly.

## Validation

- Ran release script syntax/help checks.
- Ran whitespace checks.
- Ran npm package dry-run checks.

## Release Notes

- Exact-version releases now work when `Cargo.toml` already matches the requested version.
