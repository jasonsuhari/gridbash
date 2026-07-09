# Startup update check

Date: 2026-07-09
Release target: unreleased

## Summary

- Added a startup update notice for npm-launched GridBash installs.

## What Changed

- The npm launcher now checks the latest GitHub release before starting the packaged binary.
- The check is skipped for `--version`, help output, `--mcp`, non-TTY stderr, or when `GRIDBASH_NO_UPDATE_CHECK` is set.
- Update-check failures are silent, and the launcher still starts GridBash normally.
- Added focused launcher tests for version comparison, skip conditions, timeout parsing, and the local HTTP release-check path.

## Why It Matters

- Local installs can drift behind the latest release. A concise startup notice helps users notice available updates without blocking normal startup.

## Validation

- `npm run test:launcher`
- `node --check npm/bin/gridbash.js`
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- `cargo build --release`
- `node npm/scripts/prepare.js`
- `npm pack --dry-run --ignore-scripts`
- `GRIDBASH_ALLOW_WORKTREE_LINK=1 node npm/bin/gridbash.js --version`

## Release Notes

- GridBash now reports when a newer GitHub release is available at startup, while keeping protocol and version/help output clean.
