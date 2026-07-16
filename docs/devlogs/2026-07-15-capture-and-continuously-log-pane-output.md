# Capture and continuously log pane output

Date: 2026-07-15
Release target: unreleased

## Summary

- Added bounded pane-output capture and opt-in continuous plain-text logging.

## What Changed

- Added Alt+Shift+C capture and Alt+Shift+L logging controls for the focused or
  selected panes.
- Added collision-safe per-pane output files with visible logging badges and
  resolved-path status messages.
- Added capture, start-log, and stop-log tools to the local agent control API.
- Isolated log write failures so one failed destination cannot interrupt PTY
  parsing or sibling panes.

## Why It Matters

- Agent debugging, build evidence, and long-running command output can be saved
  without manual selection or mixing unrelated pane input into the record.

## Validation

- `cargo fmt -- --check`
- `cargo test output_capture`
- `cargo test control`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`

## Release Notes

- Capture recent pane output or maintain continuous per-pane logs from the TUI
  and local control API.
