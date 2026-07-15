# Make GridBash shortcuts configurable

Date: 2026-07-15
Release target: unreleased

## Summary

- Made every modeless GridBash keyboard action configurable.

## What Changed

- Added a validated `[keys]` configuration table with stable action names and
  normalized Ctrl/Alt/Shift chords.
- Replaced hard-coded application dispatch with one typed action map while
  keeping ordinary unbound keys in the child terminal.
- Made in-app help render the effective bindings.
- Reserved F1 and `Alt+q` as reliable help and quit recovery paths.
- Updated the example config, README, and reference.

## Why It Matters

- Users can resolve host-terminal conflicts and adapt GridBash to an existing
  muscle-memory layout without sacrificing terminal passthrough or recovery.

## Validation

- `cargo fmt -- --check`
- `cargo test keybindings`
- `cargo test` with `GRIDBASH_INVOKING_PROFILE` removed
- `cargo clippy --all-targets -- -D warnings`

## Release Notes

- Add overrides under `[keys]`; press F1 at any time to see the effective map.
