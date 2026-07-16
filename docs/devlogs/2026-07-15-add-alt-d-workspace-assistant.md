# Add Alt D workspace assistant

Date: 2026-07-15
Release target: unreleased

## Summary

- Added a conversational BashBot workspace assistant behind Alt+D.
- Tracks issue #217.

## What Changed

- Added a bottom-right avatar dock with editable chat input, history, and clear
  ready, busy, configuration, and error states.
- Sends bounded, labeled context from every pane in every open grid to the
  configured Manager API.
- Supports workspace briefs and prompt coaching without dispatching input.
- Allows explicit delegation requests to submit validated, single-line prompts
  to live panes across grids.
- Binds assistant targets to stable PTY identities and skips commands when a
  pane changed, slept, exited, or disappeared during review.
- Documented Alt+D in the in-app help, README, and reference.

## Why It Matters

- GridBash now has one friendly place to understand the whole workspace instead
  of inspecting tabs and panes individually.
- Explicit dispatch intent and stale-target checks keep conversational help
  separate from pane mutation.

## Validation

- `npm test` (176 passed, 1 ignored interactive ConPTY test)
- `cargo clippy --all-targets -- -D warnings`
- `npm run test:launcher` (11 passed)
- `npm run test:review-agent` (9 passed)
- `npm run test:issue-labeler` (19 passed)
- `npm run test:star-history` (5 passed)
- `npm run test:version` (4 passed)
- `cargo fmt --all -- --check`
- `git diff --check`

## Release Notes

- Press Alt+D to ask BashBot for a brief, improve a prompt, or coordinate work
  across all open grids and panes.
