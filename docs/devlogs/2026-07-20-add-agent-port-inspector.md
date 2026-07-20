# Add agent port inspector

Date: 2026-07-20
Release target: unreleased
Issue: #278

## Summary

- GridBash now surfaces agent-owned localhost listeners in a footer-backed port inspector.
- The inspector identifies the owning process and pane, and can terminate a stale development server after confirmation.

## What Changed

- Added a live `Ports N` control at the bottom-right of the workspace and a configurable `Ctrl+Alt+P` shortcut.
- Added a modal listing TCP port, process name, PID, and owning pane or tab, with mouse selection and keyboard navigation.
- Added asynchronous Windows, macOS, and Linux listener discovery by matching socket PIDs to authenticated pane-host ancestry.
- Added guarded termination that re-scans and verifies ownership immediately before signaling the process.

## Why It Matters

- Coding agents frequently start development servers and leave them running. The inspector makes port conflicts visible without leaving GridBash or guessing which pane owns a process.
- Filtering by pane-host ancestry keeps unrelated system services and GridBash's private control listeners out of the termination surface.

## Validation

- `cargo fmt --all -- --check`
- Focused port parser, ancestry, shortcut, state, and footer layout tests.
- Repository Cargo validation under the shared integration lease.

## Release Notes

- Added an agent port inspector with safe process termination from the GridBash footer.
