# Restart exited terminal panes

Date: 2026-07-07
Release target: unreleased

## Summary

- Added an in-app recovery path for panes whose terminal process has exited.
- Tracks #79.

## What Changed

- Added `Alt+t` to restart exited focused panes, or exited selected panes when multiple panes are selected.
- Respawned panes reuse their original launch spec, cwd, profile, and worktree metadata.
- Typing into an exited pane now shows a restart hint instead of trying to write into a dead PTY.
- Updated the in-app footer/status hints and README controls table.

## Why It Matters

- When an agent exits after noisy Windows cleanup output, the pane previously looked like a broken terminal with no recovery action.
- Users can now revive the pane in place without rebuilding the whole GridBash session.

## Validation

- Added targeted unit coverage for selecting only exited panes as restart candidates.
- Ran `cargo test`.

## Release Notes

- New recovery shortcut: press `Alt+t` to restart exited terminal panes.
