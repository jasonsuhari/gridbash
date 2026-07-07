# Startup picker and release automation

Date: 2026-07-07
Release target: unreleased

## Summary

- Replaced the old setup wizard with a fullscreen startup grid picker.
- Added release/devlog automation and a CI/CD release path.
- Fixed local install drift so the global `gridbash` command uses a packed copy instead of a live worktree.

## What Changed

- Startup now uses the current working directory automatically.
- The old work-folder selection, vibe-profile selection, and launch-preview steps are gone.
- The picker defaults to a 2 row x 3 column grid and updates the preview as dimensions change.
- Preview panes render as square cells with green borders and green terminal-color fill.
- Windows CWD display strips extended path prefixes like `\\?\`.
- `npm run install:local` installs from a packed tarball and guards against source-checkout global installs.
- Devlogs and release notes are included in the npm package.
- The `Release` GitHub Actions workflow can prepare the version bump, publish npm, and create the GitHub release from CI/CD.

## Why It Matters

- Starting GridBash is now a direct grid-size choice instead of a multi-step setup flow.
- The installed CLI is less likely to silently point at stale worktrees or old branches.
- Release notes can be generated and shipped with the package for npm and GitHub.
- A coding agent can run the release from GitHub Actions after reviewing and merging the PR.

## Validation

- Ran Rust formatting, linting, and tests.
- Ran release script syntax/help checks.
- Ran npm package dry-run checks to confirm docs/devlogs are included.
- Reinstalled the CLI locally from the packed npm package and verified `gridbash --version`.

## Release Notes

- New fullscreen startup grid picker with live 2x3 default preview.
- Removed old work-folder, profile, and launch-preview setup screens.
- Added release/devlog tooling and CI/CD publishing workflow.
- Added safer local npm install behavior for development.
