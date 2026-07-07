# Hidden manager agent groups

Date: 2026-07-07
Release target: unreleased

## Summary

- Added hidden manager agent groups for steering selected worker panes from inside GridBash.

## What Changed

- `Alt+g` now turns selected panes into a worker group, attaches a hidden manager profile, and marks the panes with a `:3` badge plus deterministic non-green tint.
- Pressing `Alt+g` on a grouped pane opens a compact prompt that sends an instruction to that group's hidden manager.
- Managers can emit fenced `gridbash send` blocks, which GridBash validates and dispatches only to panes in the manager's group.
- Worker output snapshots are relayed back to the hidden manager after output idles.
- Added `--manager-profile`, `GRIDBASH_MANAGER_PROFILE`, and `[defaults].manager_profile` resolution.

## Why It Matters

- GridBash can now coordinate groups of visible worker agents without forcing the user to manually type into each pane or dedicate a visible cell to the manager.

## Validation

- Ran `cargo fmt`.
- Ran `cargo test` with 16 passed and 1 ignored Windows ConPTY smoke test.

## Release Notes

- New hidden manager groups: select panes, press `Alt+g`, then talk to the attached manager with `Alt+g` from any grouped pane.
