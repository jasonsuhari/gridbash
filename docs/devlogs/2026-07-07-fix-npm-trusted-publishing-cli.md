# Fix npm trusted publishing CLI

Date: 2026-07-07
Release target: unreleased

## Summary

- Updated the release workflow to install the latest npm CLI before npm publish.
- Keeps the existing `NPM_TOKEN` fallback while making the tokenless trusted
  publishing path compatible with current npm requirements.

## What Changed

- Added `npm install -g npm@latest` to both GitHub release publish jobs:
  manual workflow dispatch and tag push publishing.
- Left the existing auth logic intact: use `NPM_TOKEN` when present, otherwise
  publish via npm trusted publishing/OIDC.

## Why It Matters

- The `v0.1.5` workflow built and uploaded GitHub release assets, but npm
  publish failed with `ENEEDAUTH`.
- npm trusted publishing requires a recent npm CLI. Upgrading npm inside the
  publish jobs removes runner image drift as a release blocker.

## Validation

- Run `git diff --check`.
- Open PR and require CI/DCO checks.
- Publish the fixed terminal input regression as a new patch version after this
  workflow update lands.

## Release Notes

- Release automation now refreshes npm before publishing, improving npm trusted
  publishing reliability.
