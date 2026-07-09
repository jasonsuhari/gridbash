# Dedicated command bar

Date: 2026-07-07
Release target: unreleased

## Summary

- Added a dedicated one-line command bar above the GridBash status footer.

## What Changed

- The live view now reserves a command prompt line that starts in the directory where GridBash was launched.
- Alt-arrow focus navigation can move into and out of the command line alongside panes.
- Commands run through the host shell with output captured into a hidden buffer by default.
- `Alt+e` toggles the captured command output panel, leaving `Alt+x` for pane swapping.
- Built-in `cd`, `pwd`, `clear`, and `cls` behavior keeps the command cwd useful without requiring a persistent shell.
- The command line was ported onto the current grid behavior without regressing pane sleep, swap, mouse wake, usage labels, runtime resize, or worktree labels.

## Why It Matters

- Users can run quick workspace commands without stealing input from a live agent pane or dedicating a full pane to command output.

## Validation

- `npm test`

## Release Notes

- Adds a focused command bar with hidden output capture and Alt-key output expansion.
