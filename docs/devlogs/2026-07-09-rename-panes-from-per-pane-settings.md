# Rename panes from per-pane settings

Date: 2026-07-09
Release target: unreleased

## Summary

- Added pane renaming directly to the focused pane settings overlay.

## What Changed

- Added a visible `Rename pane` action alongside history reload.
- Added `N` and `Alt+r` keyboard access plus mouse activation.
- Reused the existing rename editor so validation and saved session behavior stay consistent.

## Why It Matters

- Pane names can now be changed where pane-specific details are already managed, making the feature easier to discover.

## Validation

- `cargo fmt -- --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `npm run test:launcher`

## Release Notes

- Pane settings now includes a rename action with keyboard and mouse support.
