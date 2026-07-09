# Deselect specific panes

Date: 2026-07-09
Release target: unreleased

## Summary

- Added direct right-click toggling for individual pane selection.

## What Changed

- Right-clicking a pane now selects it when it is unselected.
- Right-clicking an already selected pane now deselects only that pane.
- The existing `Alt+s` focused-pane toggle now reports whether the pane was selected or deselected.
- The controls docs and in-app status hint now mention right-click pane toggling.

## Why It Matters

- Users can start from a broad selection, such as all panes, and remove one specific pane without clearing or rebuilding the rest of the selection.

## Validation

- Ran `cargo fmt --check`.
- Ran `cargo test toggle_selection`.
- Ran `cargo test`.
- Ran `cargo clippy -- -D warnings`.

## Release Notes

- Right-click a pane to toggle it in or out of the current selected pane set.
