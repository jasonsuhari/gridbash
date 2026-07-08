# Integrate tabbed grids

Date: 2026-07-07
Release target: unreleased

## Summary

- Ported the old tabbed grids branch onto current `main` without dropping newer pane controls.

## What Changed

- Added per-tab grid state for panes, selection, sleep state, pane names, text selection, layout, and launch plans.
- Added a tab strip with `Alt+t` next tab, `Alt+n` new tab, and `Alt+Shift+r` tab rename.
- Kept `Alt+r` for focused pane rename so current pane rename behavior is preserved.
- Added OSC 7 cwd tracking so new tabs can open from the active pane's current folder.

## Why It Matters

- Users can keep multiple independent GridBash grids alive in one terminal while retaining runtime resize, sleep, pane selection, pane rename, worktree labels, usage labels, and conversation footers.

## Validation

- `npm test`

## Release Notes

- Add tabbed grids with a top tab bar and non-conflicting tab shortcuts.
