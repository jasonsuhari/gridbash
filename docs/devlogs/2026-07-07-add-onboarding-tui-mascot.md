# Add onboarding TUI mascot

Date: 2026-07-07
Release target: unreleased

## Summary

- Added BashBot, a small ASCII TUI mascot, to the first-run GridBash setup/onboarding flow.

## What Changed

- The loading and terminal-profile picker screens now reserve a responsive mascot area on wider terminals.
- Narrow terminals fall back to a compact BashBot badge so setup remains usable without requiring a large window.
- The loading progress gauge and terminal picker content continue to render inside the existing setup panels.

## Why It Matters

- First-run setup now feels more distinctive while keeping the same terminal selection behavior and keyboard flow.

## Validation

- `cargo fmt --check`
- `cargo test`
- `cargo clippy -- -D warnings`

## Release Notes

- Adds a responsive BashBot mascot to the setup/onboarding TUI.
