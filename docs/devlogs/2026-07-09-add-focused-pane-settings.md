# Add focused pane settings

Date: 2026-07-09
Release target: unreleased

## Summary

- Replaced the `Alt+p` previous-panes picker with a focused-pane settings view.

## What Changed

- `Alt+p` now opens settings inside the focused pane instead of drawing a global picker modal.
- The previous panes selector remains available from its button and `Alt+Shift+p`.
- Added a `Reload past history` action that refreshes the pane's visible conversation snapshot.
- Kept `Alt+o` dedicated to overall GridBash settings, including from the pane settings view.

## Why It Matters

- Pane-specific controls now live where the user is already looking, and the old redundant pane picker no longer competes with existing focus shortcuts.

## Validation

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo clippy -- -D warnings`

## Release Notes

- Changed `Alt+p` to open focused-pane settings with a `Reload past history` action.
