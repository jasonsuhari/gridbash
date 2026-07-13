# Complete starter UX controls

Date: 2026-07-12
Release target: unreleased

## Summary

- Made settings durable, profile diagnostics actionable, onboarding persistence
  testable, and keyboard controls discoverable inside GridBash.

## What Changed

- Added an `Alt+h`/F1 help overlay with responsive shortcut columns.
- Expanded `--list-profiles` with default, source, resolved-path, and missing-command
  diagnostics without exposing credentials.
- Persisted compact titles, activity badges, quit confirmation, scrollback,
  refresh delay, and palette choices under `[ui]`.
- Wired compact titles, two-step quit confirmation, scrollback for new panes, and
  refresh delay to real runtime behavior; removed the nonfunctional pane-density row.
- Added deterministic coverage proving a clean first-run config saves its selected
  terminal and skips onboarding on the next launch.

## Why It Matters

- Users can discover controls without leaving the TUI, understand why profiles are
  missing or selected, and trust that settings labeled as controls survive restart.

## Validation

- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt -- --check`
- `npm run test:launcher`
- `git diff --check`

## Release Notes

- Added in-app help, durable UI settings, detailed profile diagnostics, and
  clean-profile onboarding regression coverage.
