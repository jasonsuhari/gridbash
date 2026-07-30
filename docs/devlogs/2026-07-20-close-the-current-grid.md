# Close the current grid

Date: 2026-07-20
Release target: unreleased

## Summary

- Added a safe way to close one grid without quitting the whole GridBash workspace.

## What Changed

- Added the configurable `close-grid` action with the default `Alt+w` shortcut.
- Added a confirmation dialog that names the grid and pane count before stopping processes.
- Terminated pane hosts, cleared grid-local runtime state, activated an adjacent grid, and saved the updated session.
- Prevented the only remaining grid from being closed.

## Why It Matters

- Multi-grid sessions can retire finished work without rebuilding or exiting the rest of the workspace.

## Validation

- Focused Rust tests for action matching, shortcuts, close selection, confirmation input, and dialog rendering.
- Repository formatting, lint, and test checks before merge.

## Release Notes

- Press `Alt+w`, then Enter or Y, to close the current grid. Press Escape or N to cancel.
