# Show conversation summaries in pane footers

Date: 2026-07-07
Release target: unreleased

## Summary

- Agent panes now replace the plain bottom border run with a compact conversation footer.

## What Changed

- Added launch-spec detection for Claude, Codex, other builtin agent profiles, custom agent commands, and `vibe run <agent> --` panes.
- Added a visible-screen summarizer that pulls the latest meaningful conversation line from agent PTYs.
- Rendered the summary as a pane bottom title so it occupies the footer border line without reducing terminal content height.

## Why It Matters

- Multi-agent grids are easier to scan because each coding pane exposes its latest visible context without forcing focus changes.

## Validation

- `cargo fmt --check`
- `cargo test`
- `cargo clippy -- -D warnings`

## Release Notes

- Agent panes now show a compact conversation summary in their footer line.
