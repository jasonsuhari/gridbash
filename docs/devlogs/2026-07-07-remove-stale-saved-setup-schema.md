# Remove stale saved setup schema

Date: 2026-07-07
Release target: unreleased

## Summary

- Removed the stale saved-setup config schema left behind after the startup picker replaced the old composer flow.
- Kept the current startup picker, direct launch, and managed worktree launch behavior intact.

## What Changed

- Removed `Config::setups` and the unused `save_setup` helper.
- Removed obsolete saved setup types and helper tests from `src/setup.rs`.
- Added config parsing coverage so legacy `[setups]` tables are ignored without breaking config load.
- Dropped saved workspace/templates from the roadmap until there is a fresh product design for it.

## Why It Matters

- The code now matches the current product surface: GridBash launches from the startup grid picker or explicit CLI options, not named saved setups.
- Users with old config files can keep starting GridBash while the unused setup table is ignored.

## Validation

- `npm test` (43 passed, 1 ignored)

## Release Notes

- Removed stale saved-setup internals from config parsing and setup planning.
