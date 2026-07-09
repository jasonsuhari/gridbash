# Fix pane wheel scrolling

Date: 2026-07-09
Release target: unreleased

## Summary

- Fixed mouse wheel scrolling for terminal apps running inside GridBash panes.

## What Changed

- GridBash now treats wheel events as pane mouse input instead of ignoring them while mouse capture is enabled.
- Wheel events focus the hovered pane and are forwarded using the child terminal's active xterm mouse protocol.
- Plain shells are protected from stray escape text because scroll bytes are only sent when the child app has enabled mouse tracking.
- Added unit coverage for disabled mouse mode, SGR mouse encoding, and default mouse encoding.

## Why It Matters

- Agent TUIs such as Codex, Claude, and similar tools can receive scroll wheel input inside their own panes instead of appearing stuck behind GridBash's mouse capture.

## Validation

- `cargo test mouse_scroll`
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- `cargo build --release`

## Release Notes

- Fixes pane-local mouse wheel scrolling for terminal apps that request mouse tracking.
