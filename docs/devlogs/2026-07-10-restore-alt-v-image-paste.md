# Restore Alt+v image paste

Date: 2026-07-10
Release target: unreleased
Issue: #138

## Summary

- Moved voice mode to `Alt+Shift+V` so agent panes can receive plain `Alt+v` for clipboard-image paste.

## What Changed

- Restricted voice start and cancellation to `Alt+Shift+V`.
- Allowed plain `Alt+v` to pass through to the focused terminal pane.
- Updated runtime hints and voice documentation to show the new shortcut.
- Added regression coverage for both shortcut paths.

## Why It Matters

- Codex and Claude users can paste clipboard images without losing GridBash voice input.

## Validation

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo build --release`

## Release Notes

- Voice mode now uses `Alt+Shift+V`; plain `Alt+v` once again reaches focused agent panes for clipboard-image paste.
