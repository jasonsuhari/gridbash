# Fix exited pane recovery dialog

Date: 2026-07-09
Release target: unreleased

## Summary

- Added an obvious in-app recovery dialog for focused panes whose terminal process has exited.

## What Changed

- Focused exited panes now show a modal with restart and sleep actions when no other modal is open.
- Pressing `Enter`, `r`, or `t` in the dialog restarts the focused exited pane.
- Pressing `z` or `s` in the dialog puts the focused exited pane to sleep.
- The dialog holds a leading `Esc` key so terminals that encode `Alt+t` as `Esc` then `t` can still restart an exited focused pane.
- Updated README controls to document the recovery dialog.

## Why It Matters

- Exited panes no longer rely on a hidden shortcut or status bar hint for recovery.
- The restart path now works in the common terminal encoding where Alt shortcuts arrive as an escape-prefixed character sequence.

## Validation

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- `cargo build --release`

## Release Notes

- Exited panes now show a restart/sleep recovery dialog when focused.
