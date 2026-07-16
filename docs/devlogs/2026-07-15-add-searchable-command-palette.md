# Add searchable command palette

Date: 2026-07-15
Release target: unreleased

## Summary

- Added a searchable, keyboard-first action palette inspired by Zed's command surface.

## What Changed

- `Alt+k` opens a modal list of GridBash pane, tab, grid, manager, settings, help, and quit actions.
- Typing or pasting filters actions with tolerant subsequence matching; Up/Down and Enter navigate and execute.
- Moved the modeless shortcut definitions into a typed action registry shared by direct keyboard dispatch and the palette.

## Why It Matters

- GridBash has grown beyond a handful of shortcuts. The palette makes existing capabilities discoverable without sending search text into a live PTY.

## Validation

- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`

## Release Notes

- Open the new command palette with `Alt+k`, search by action or intent, and press Enter to run the selected command.
