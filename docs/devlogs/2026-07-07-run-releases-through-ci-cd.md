# Run releases through CI CD

Date: 2026-07-07
Release target: unreleased

## Summary

- Move release preparation into GitHub Actions.
- Keep tag-based npm/GitHub publishing intact.
- Track this request through GitHub issue #14.

## What Changed

- Added `workflow_dispatch` inputs to the `Release` workflow.
- Added a `prepare` job that runs `node npm/scripts/release.js` on `main`.
- Added a separate manual publish job that builds from the created tag.
- Made npm publishing work with either an `NPM_TOKEN` secret or npm Trusted Publishing/OIDC.
- Made npm and GitHub release publishing retry-friendly when the package version or release already exists.
- Kept the tag-triggered publish path for local release fallbacks.
- Updated release docs to make GitHub Actions the normal release path.

## Why It Matters

- A coding agent can start a release from CI/CD without depending on local shell state.
- Version bump commits and tags are created by GitHub Actions with the bot identity.
- Publishing can happen fully inside the manual GitHub Actions run.
- Failed publish jobs can be retried without re-running the version bump job.
- Local fallback tags still publish through the tag event path.

## Validation

- Reviewed the workflow split between manual CI/CD release and fallback tag publishing.
- Reviewed the retry path for publish failures after a tag is created.
- Ran syntax checks and package validation before merge.

## Release Notes

- Releases can now be prepared from the GitHub Actions `Release` workflow.
- Manual releases publish from a separate job after the tag is created.
- Tag pushes continue to publish npm and create GitHub releases.
