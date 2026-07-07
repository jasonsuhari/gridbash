# Swap Selected Tiles

Date: 2026-07-07
Release target: unreleased

## Summary

- Added an `Alt+x` command for swapping exactly two selected panes.

## What Changed

- `Alt+x` now swaps the two selected pane positions.
- The command reports `select two panes to swap` when fewer than two panes are selected.
- The command reports `deselect panes until only two are selected` when more than two panes are selected.
- Swapping preserves selected positions while moving the focused pane, sleeping state, and launch labels with the swapped pane content.

## Why It Matters

- Users can rearrange pane positions without restarting a grid or rebuilding a setup.

## Validation

- Ran `cargo fmt --check`.
- Ran `cargo test swap`.
- Ran `cargo test`.
- Ran `cargo clippy -- -D warnings`.

## Release Notes

- Added `Alt+x` to swap exactly two selected panes.
