# Fix release publish retry

Date: 2026-07-07
Release target: unreleased

## Summary

- Fixed the release publish retry path after `v0.1.4` prepared successfully but failed while packing npm assets.
- Tracked the retry fix through GitHub issue #32.

## What Changed

- The manual release workflow now detects an existing exact-version tag and skips release preparation.
- The publish jobs now parse the tarball filename from `npm pack --json` even when npm lifecycle output appears before the JSON.
- The same robust pack parsing is used for manual publish retries and tag-triggered publishing.

## Why It Matters

- A failed publish job can be retried without recreating the release commit or tag.
- npm pack output differs between local runs and GitHub Actions when lifecycle scripts emit output.
- The already-created `v0.1.4` tag can now be published from CI/CD.

## Validation

- Reviewed the existing-tag dispatch path for exact versions.
- Ran workflow/doc whitespace checks.
- Planned CI validation before merge and release rerun.

## Release Notes

- Release workflow retries now handle existing exact-version tags and noisy npm pack output.
