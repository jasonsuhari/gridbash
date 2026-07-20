# Unify BashBot Director command center

Date: 2026-07-20
Release target: unreleased

## Summary

- Unified the command line, BashBot chat, and manager goals behind Alt+C.
- Tracks issue #284.

## What Changed

- Added per-grid Chat and Shell modes in a resizable bottom command center.
- Added `/goal` and `/stop` for continuous grid supervision with changed-pane
  status digests and delivery receipts in the transcript.
- Kept targeted prompts bound to stable pane identities and revision snapshots.
- Routed late chat, voice, and shell results back to the grid that started them.
- Removed the Alt+D, Alt+G, and Alt+U defaults and added clear migration errors
  for their former `[keys]` action names.
- Kept transcripts and active goals memory-only instead of writing them into
  recoverable session files.

## Why It Matters

- One surface now explains pane activity, delegates follow-ups, supervises a
  goal, and runs host-shell commands without mixing scopes across grids.

## Validation

- `cargo fmt --all -- --check`
- `git diff --check`
- Final Cargo validation pending the shared integration lease.

## Release Notes

- Alt+C now opens BashBot Director with per-grid Chat and Shell modes, changed
  pane updates, and explicit continuous goal supervision.
