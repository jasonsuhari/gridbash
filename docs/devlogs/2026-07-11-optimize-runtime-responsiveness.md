# Optimize Runtime Responsiveness

Date: 2026-07-11
Release target: unreleased

## Summary

- Made terminal input and the desktop remain responsive under large, busy grids.

## What Changed

- Bounded PTY output buffering and limited each event-loop drain by event count,
  byte count, and elapsed time so output cannot starve keyboard handling.
- Moved pane writes onto ordered background workers so a blocked child cannot
  freeze the GridBash interface.
- Added revision-based screen and conversation-summary caches, a 30 FPS output
  cap for large grids, and suppression of redundant redraws from sleeping and
  inactive panes.
- Added adaptive Windows Job Object scheduling. Focused and selected panes receive
  more CPU time under contention while all background panes keep running.
- Reduced output allocations and made history trimming amortized instead of
  rescanning a full tail after every chunk.

## Why It Matters

- Twenty or more active panes can generate output faster than a terminal UI can
  parse and repaint it. Bounded work and adaptive scheduling keep that load from
  turning into input latency or an unusable desktop.

## Validation

- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `npm run test:launcher`
- `cargo build --release`

## Release Notes

- Strongly improved input latency and system responsiveness for large grids while
  preserving pane output, appearance, and background progress.
