# Add a scriptable CLI for running grid sessions

Date: 2026-07-15
Release target: unreleased

## Summary

- Added `gridbash ctl` for discovering, inspecting, and safely controlling
  opted-in running grids from scripts.

## What Changed

- Published owner-local runtime discovery metadata without bearer credentials.
- Added human and JSON `ctl list` and `ctl panes` output.
- Added token-authenticated send, capture, status, and focus commands.
- Added stable pane identities with generation checks that reject stale targets.
- Reused the same serialized control commands for MCP tools and the CLI.

## Why It Matters

- External scripts can discover the right grid, inspect exact pane state, and
  perform bounded actions without guessing ports or racing pane restarts.

## Validation

- `cargo fmt -- --check`
- `cargo test control`
- `cargo test control_discovery`
- `cargo test cli`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`

## Release Notes

- Use `gridbash ctl` for machine-readable running-session and pane control.
