# Configurable pane border color

Date: 2026-07-09
Release target: unreleased

## Summary

- Made quiet pane borders muted by default and kept pane chrome colors editable in settings.

## What Changed

- Changed the default quiet-output pane border from magenta to dark gray.
- Removed bold styling from quiet-output borders so panes with the `*` quiet marker match the muted idle chrome by default.
- Added dark gray to the runtime palette choices so users can cycle quiet/selected/focus colors through a muted border option.
- Updated tests for settings palette rows and quiet pane chrome.

## Why It Matters

- Quiet panes should not look selected or urgent just because output paused. The `*` marker still communicates quiet output while the border remains visually consistent unless the user chooses a brighter color.

## Validation

- TODO before merge: `cargo fmt --check`
- TODO before merge: `cargo clippy -- -D warnings`
- TODO before merge: `cargo test`

## Release Notes

- Quiet pane borders now default to muted gray instead of magenta, and the settings palette includes dark gray as an editable pane chrome color.
