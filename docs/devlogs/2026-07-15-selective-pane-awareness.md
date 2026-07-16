# Selective pane awareness

Date: 2026-07-15
Release target: unreleased

## Summary

- Added pull-based, read-only awareness tools so agents can inspect sibling pane
  activity only when coordination requires it.
- Tracks issue #230.

## What Changed

- Added `gridbash_get_grid_snapshot` for current-grid pane metadata, state, and
  activity summaries without dumping complete transcripts.
- Added `gridbash_read_pane_output` for explicit, bounded reads of recent output
  from up to eight available panes.
- Gave each live pane a stable control ID so reads remain attached to the same
  PTY across grid reordering while snapshots still show current pane numbers.
- Labeled all peer summaries and output as untrusted context in MCP guidance and
  responses, and rejected sleeping, exited, stale, unknown, or oversized reads.
- Kept the awareness API opt-in behind `--agent-api` and left existing mutating
  tools unchanged.

## Why It Matters

- Independent agents can discover relevant sibling work at handoff, dependency,
  conflict, or integration points without continuously sharing every transcript
  or consuming peer context by default.

## Validation

- `cargo test --no-fail-fast -j 1` with inherited GridBash profile variables
  cleared (174 passed, 1 ignored interactive ConPTY test)
- `cargo clippy --all-targets -j 1 -- -D warnings`
- `npm test` (174 passed, 1 ignored interactive ConPTY test)
- `npm run test:launcher` (11 passed)
- `npm run test:review-agent` (9 passed)
- `npm run test:issue-labeler` (19 passed)
- `npm run test:star-history` (5 passed)
- `npm run test:version` (4 passed)
- `cargo fmt --all -- --check`
- `git diff --check`

## Release Notes

- Agents can now pull lightweight grid snapshots and bounded peer output through
  GridBash's opt-in MCP control API.
