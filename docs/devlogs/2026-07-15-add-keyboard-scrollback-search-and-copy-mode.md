# Add keyboard scrollback search and copy mode

Date: 2026-07-15
Release target: unreleased

## Summary

- Added a keyboard-first viewer for searching, selecting, and copying the
  focused pane's bounded terminal history.

## What Changed

- Added Alt+B copy mode with arrows, Home/End, and page navigation.
- Added incremental `/` search with next/previous match navigation.
- Added character and whole-line selection with clipboard copy through the
  existing OSC 52 and macOS paths.
- Kept the viewer snapshot stable while live PTY output continues behind it.

## Why It Matters

- Keyboard-first users can locate and reuse earlier agent output without
  reaching for the mouse or disturbing sibling panes.

## Validation

- `cargo fmt -- --check`
- `cargo test copy_mode`
- `cargo test history_snapshot`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`

## Release Notes

- Press Alt+B to search, select, and copy focused-pane scrollback from the
  keyboard.
