# Add grid session resume command

Date: 2026-07-07
Release target: unreleased

## Summary

- Added `gridbash resume` for finding and reopening saved GridBash sessions.

## What Changed

- Added a session snapshot store under GridBash local app data.
- Normal grid launches now save bounded session metadata for the grid, pane profiles, working directories, labels, worktree names, submitted command history, and recent output context.
- Added `gridbash resume`, `gridbash resume --latest`, `gridbash resume --list`, and `gridbash resume <session-id>`.
- Resumed sessions relaunch the saved grid and show per-pane history context without replaying old commands into child shells.

## Why It Matters

- Agent-heavy work often spans multiple panes and folders. Resume makes prior GridBash grids discoverable as whole workspaces instead of forcing users to rebuild layout and context by hand.

## Validation

- `cargo fmt --check`
- `cargo test`
- `cargo clippy -- -D warnings`

## Release Notes

- Add `gridbash resume` to reopen saved grids with per-pane command and output history context.
