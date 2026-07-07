# Rename panes

Date: 2026-07-07
Release target: unreleased

## Summary

- Added focused-pane renaming so custom labels can replace the numeric pane prefixes during a GridBash session.

## What Changed

- Added an `Alt+r` rename modal with editable text, `Enter` save, `Esc` cancel, and `Ctrl+u` clear.
- Pane headers now render a saved custom label in the slot previously occupied by `1`, `2`, `3`, and so on.
- Updated the README controls table with the rename shortcut and blank-name reset behavior.

## Why It Matters

- Multi-agent grids are easier to scan when panes can be named after the task, service, or agent role instead of only by position.

## Validation

- `cargo fmt --check`
- `cargo test`

## Release Notes

- Added `Alt+r` pane renaming for session-local pane labels.
