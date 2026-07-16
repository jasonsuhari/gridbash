# Background terminal jobs

Date: 2026-07-15
Release target: unreleased

## Summary

- Added a session-wide pool for keeping coding-agent terminals alive outside the visible grid.

## What Changed

- Added `Alt+Shift+B` to background selected or focused panes and atomically launch fresh replacements with the same configuration.
- Added `Alt+Ctrl+B` and a status-bar counter for browsing working, quiet, exited, and offline background agents.
- Added non-destructive insertion, confirmed live-job termination, explicit offline restart, background output routing, and saved-session metadata.

## Why It Matters

- Long-running agent work can keep progressing while its grid cell is reused, then return without either terminal being killed.

## Validation

- Pending final validation after persistent pane-host integration.

## Release Notes

- Background live terminal jobs, reuse their cells, and swap them back into view from a keyboard-first session-wide picker.
