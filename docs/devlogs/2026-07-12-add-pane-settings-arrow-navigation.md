# Add pane settings arrow navigation

Date: 2026-07-12
Release target: unreleased

## Summary

- Made the focused-pane activity controls fully navigable without reaching for their letter shortcuts or the mouse.

## What Changed

- Added a visible selection across auth, rename, history, sleep/wake, and manager-goal controls.
- Added Up/Down selection, contextual Left/Right auth choice, and Enter/Space activation.
- Kept the existing direct shortcuts and mouse controls available.

## Why It Matters

- Alt+p now opens a coherent keyboard-first activity and settings workflow instead of a collection of individually discoverable actions.

## Validation

- `cargo fmt --all -- --check`
- `cargo test pane_settings`
- `cargo test` with `GRIDBASH_INVOKING_PROFILE` removed from the child test environment
- `cargo clippy --all-targets -- -D warnings`

## Release Notes

- Pane Activity controls can now be navigated with the arrow keys and activated with Enter or Space.
