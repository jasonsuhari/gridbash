# Show auth usage in gridcell headers

Date: 2026-07-07
Release target: unreleased

## Summary

- Gridcell headers can now show remaining vibe auth usage and optional OpenAI
  API spend context while panes are running.

## What Changed

- Added a background usage monitor that resolves pane launch profiles to vibe
  profile auth directories, or the default Claude/Codex auth directories for
  direct profile launches.
- Added compact header labels for Claude/Codex usage windows, such as
  `5h 40% left / 7d 92% left`.
- Added optional OpenAI API spend labels, such as `API $1.50 24h`, when
  `OPENAI_ADMIN_KEY` is available.
- Kept all usage and spend fetches best-effort so missing auth, unsupported
  profiles, offline network, or endpoint failures leave headers unchanged.

## Why It Matters

- Users running many agent panes can see how much account capacity remains
  without leaving GridBash.
- API spend can be visible alongside pane context for users who monitor
  paid API usage during agent work.

## Validation

- `cargo test`

## Release Notes

- Gridcell headers now surface best-effort Claude/Codex auth usage and optional
  OpenAI API spend labels.
