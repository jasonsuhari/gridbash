# Runtime grid resizing

Date: 2026-07-07
Release target: unreleased

## Summary

- Add active-session shortcuts for changing the running GridBash grid shape.

## What Changed

- `Alt+r` and `Alt+c` add a row or column and spawn panes for the new cells.
- `Alt+R` and `Alt+C` shrink the grid only when the overflow panes have already exited.
- Grid layout resizing now preserves existing row and column weights.

## Why It Matters

- Users can expand a running GridBash session without restarting existing terminals.
- Shrink behavior avoids silently hiding or killing active work.

## Validation

- `cargo fmt`
- `cargo test`

## Release Notes

- Added runtime row and column controls for active GridBash sessions.
