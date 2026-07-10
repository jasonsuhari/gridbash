# Unified grid resizer

Date: 2026-07-10
Release target: unreleased

## Summary

- Replaced four incremental resize shortcuts with one full-screen grid resizer.
- Preserved pane coordinates across both expansion and contraction.

## What Changed

- `Alt+l` now opens the same row-and-column picker used during startup, initialized to the active tab's current dimensions.
- The live-grid picker renders active cells in blue and applies changes with Enter.
- Shrinking now terminates panes outside the retained upper-left rectangle, including an entire rightmost column when changing 3x3 to 3x2.
- Expanding inserts newly spawned panes into new coordinates without shifting existing rows or columns.
- Focus, selection, sleeping state, pane names, follow-ups, group membership, and launch specs follow retained panes to their new indices.
- Removed the Alt+Shift+Arrow resize handlers and documentation.

## Why It Matters

- Resizing is now visible, reversible until confirmation, and consistent with onboarding.
- Coordinate-aware remapping avoids surprising pane movement when one dimension changes or when the total cell count stays the same.

## Validation

- `cargo check --tests`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt -- --check`
- `git diff --check`

## Release Notes

- Open the unified blue grid resizer with `Alt+l`; shrinking can now deactivate live panes outside the chosen dimensions.
