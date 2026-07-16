# AI activity summaries

Date: 2026-07-15
Release target: unreleased

## Summary

- Replace unstable raw terminal-fragment pane titles with opt-in AI-written
  activity headlines backed by the existing Grid Manager API configuration.
- Keep pane output local by default and provide deterministic local activity
  states when summaries are disabled or unavailable.

## What Changed

- Added a persisted `manager.activity_summaries` privacy switch, disabled by
  default, with clear Settings copy about sending bounded active-tab output.
- Batch changed panes after quiet output, rate-limit automatic requests, pause
  during manager goals, and map responses back through stable pane identities.
- Preserve the last good headline across typing and failures. Manual Pane
  Activity refreshes bypass the cooldown while still waiting for pending input.
- Removed the raw output-tail/screen-line heuristic from pane headers, Pane
  Activity, and Previous Panes.

## Why It Matters

- Interactive terminal applications redraw prompts and chrome in small PTY
  fragments. Treating those fragments as summaries caused headers to show the
  final typed letters or Codex model/path metadata instead of useful work state.

## Validation

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `npm run test:launcher`

## Release Notes

- Pane activity headers can now show stable, concise AI work summaries without
  leaking in-progress typing into the title. The feature is explicitly opt-in
  under Settings > Manager.

Refs #229
