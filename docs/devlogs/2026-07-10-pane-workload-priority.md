# Keep Pane Workloads From Starving the Desktop

Date: 2026-07-10
Release target: unreleased

## Summary

- Run pane process trees below normal Windows priority by default.
- Keep the GridBash interface itself at normal priority.
- Allow users to restore normal pane priority through configuration.

## What Changed

- Added `[defaults].pane_priority` with `below-normal` and `normal` values.
- Applied the configured priority to each ConPTY root process after launch.
- Added Windows process-priority and config coverage.

## Why It Matters

Multiple agent panes can launch several parallel compilers at once. Giving those
workloads the same priority as interactive apps can make terminal input, browsers,
and the rest of Windows feel unresponsive even when GridBash itself is lightweight.

## Validation

- `cargo fmt -- --check`
- `cargo test`

## Release Notes

Pane workloads now run below normal Windows priority by default to protect desktop
responsiveness under heavy multi-pane CPU load. Set
`[defaults].pane_priority = "normal"` to retain the previous behavior.
