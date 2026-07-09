# Restore startup grid picker

Date: 2026-07-09
Release target: unreleased

## Summary

- Restored the no-argument startup grid picker so the installed command matches the README and release notes.
- Fixes #114.

## What Changed

- Replaced the stale multi-step composer flow with the fullscreen row/column picker.
- Restored the live 2x3 default preview, numeric dimension controls, and direct launch plan creation from the configured default profile.
- Removed stale guided-composer setup helpers that were no longer used.

## Why It Matters

- Running `gridbash` now starts with the expected grid-size choice instead of the older folder/profile/preview wizard.
- Users can see and adjust the pane layout before spawning terminals.

## Validation

- `cargo fmt --check`
- `cargo test`
- `npm run test:launcher`

## Release Notes

- Plain `gridbash` once again opens the startup grid picker with a live preview when no direct launch arguments are supplied.
