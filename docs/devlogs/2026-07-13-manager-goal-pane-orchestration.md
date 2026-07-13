# Manager goal pane orchestration

Date: 2026-07-13
Release target: unreleased

## Summary

- Expanded manager goals from pane-local reviewers into orchestrators for the
  current grid.
- Tracks issue #187.

## What Changed

- Added pane-numbered context from relevant live panes so the manager can review
  progress across the grid.
- Added validated, targeted follow-up dispatch so one manager decision can send
  distinct instructions to one or more panes.
- Kept goal routing bound to stable PTY identities across pane reordering and
  skipped sleeping, exited, or otherwise unavailable targets safely.
- Rejects stale reviews when pane input/output changes, and validates manager
  replies as bounded, single-line, one-command-per-pane dispatch batches.
- Carries partial-dispatch results into the next review and stops after bounded
  repeated failures instead of retrying forever.
- Updated manager-goal copy in the README and TUI to describe grid orchestration.

## Why It Matters

- A manager goal now matches the workflow its name implies: coordinating agents
  across the grid instead of nudging only the pane where the goal was created.
- Explicit target validation keeps orchestration visible and prevents commands
  from crossing into unintended panes.

## Validation

- `cargo test --no-fail-fast` (138 passed, 1 ignored interactive ConPTY test)
- `cargo clippy --all-targets -- -D warnings`
- `npm test` (138 passed, 1 ignored interactive ConPTY test)
- `npm run test:launcher` (11 passed)
- `npm run test:review-agent` (9 passed)
- `cargo fmt --all -- --check`
- `git diff --check`

## Release Notes

- Manager goals now review live pane output across the current grid and send
  targeted follow-ups to the panes that need them.
