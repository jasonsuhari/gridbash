# Enable pane scrollback

Date: 2026-07-09
Release target: unreleased

## Summary

- Added direct mouse-wheel scrollback inside individual GridBash panes.

## What Changed

- Routed vertical wheel events over plain shells into each pane's existing terminal scrollback buffer.
- Preserved mouse-wheel forwarding for child applications that enable terminal mouse reporting.
- Hid the live cursor while reviewing history and returned targeted panes to live output when keyboard input is sent.

## Why It Matters

- Earlier pane output can be reviewed in place without switching to Ctrl+T transcript mode.
- Each pane keeps an independent scroll position, so reviewing one does not disturb the others.

## Validation

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo build --release`
- `npm run test:launcher`

## Release Notes

- Mouse wheel and trackpad scrolling now review scrollback directly inside the pane under the pointer.

Tracks #124.
