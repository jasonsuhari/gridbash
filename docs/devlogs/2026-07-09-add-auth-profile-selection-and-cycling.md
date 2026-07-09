# add auth profile selection and cycling

Date: 2026-07-09
Release target: unreleased

## Summary

- Added GridBash-owned auth storage, per-pane auth account switching, and optional round-robin auth assignment for Claude and Codex panes.

## What Changed

- Changed the default auth profile home from `%USERPROFILE%\.claude-profiles` to `%USERPROFILE%\.gridbash-auth`, while retaining the legacy environment override for explicit compatibility.
- Added a manual-by-default `auth.auto_cycle` setting and an Auth Settings control that toggles round-robin assignment across ready profiles.
- Added a compatible account picker to Pane Settings. Applying a selection updates the pane launch environment, restarts only that pane, refreshes usage monitoring, and saves the selection in session metadata.
- Keyed pane usage labels by the applied auth directory so panes using the same launch profile but different accounts show the correct account usage.
- Documented migration choices for existing profile directories and the new global/per-pane controls.

## Why It Matters

- Auth storage now belongs clearly to GridBash instead of appearing Claude-specific.
- Users with several Claude or Codex accounts can either pin accounts pane by pane or distribute new panes automatically without changing shell environment variables by hand.

## Validation

- `cargo test` (101 passed, 1 ignored after merging current `main`)
- `cargo clippy --all-targets -- -D warnings`
- `npm test` (101 passed, 1 ignored)

## Release Notes

- GridBash auth profiles now default to `%USERPROFILE%\.gridbash-auth`.
- Choose an auth account from Pane Settings, or enable account auto-cycle from the global Auth Settings tab.

Tracks #118.
