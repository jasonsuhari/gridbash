# Pane-local manager goals and controls

Date: 2026-07-10
Release target: unreleased

## Summary

- Replaced cross-pane hidden manager groups with isolated goals owned by one pane.

## What Changed

- Added sleep/wake, create/edit goal, and stop-goal controls to focused-pane settings.
- Added a Manager settings tab for an OpenAI-compatible endpoint, model, and masked API key.
- Bound review responses to the owning pane's stable PTY identity, including across tab switches and pane reordering.
- Removed the manager-profile CLI/config flow and cross-pane routing protocol.

## Why It Matters

- Manager automation can no longer observe or route instructions across sibling panes.
- Users can configure and control the complete workflow without leaving GridBash.

## Validation

- `cargo test --no-fail-fast`
- `cargo clippy --all-targets -- -D warnings`
- `npm test`

## Release Notes

- Pane managers now act as isolated per-pane goals configured from GridBash settings.
