# Deactivate panes and compact the grid

Date: 2026-07-15
Release target: unreleased

## Summary

- Added a focused-pane deactivation control that automatically compacts the
  live grid.

## What Changed

- Added a Deactivate pane action and `d` shortcut to Pane Activity.
- Removed deactivated pane processes and compacted pane-indexed state without
  disturbing the remaining terminals.
- Shrunk grid dimensions column-first whenever the remaining pane count fits;
  a `2x3` grid with four panes now becomes `2x2`.
- Kept the final pane protected so every tab retains a usable terminal.

## Why It Matters

- GridBash no longer leaves a hole where a pane was deactivated.
- Dense grids reclaim screen space automatically instead of requiring a
  separate resize pass.

## Validation

- `cargo fmt --all -- --check`
- Focused pane compaction and settings UI tests
- Full Rust test suite

## Release Notes

- Open Pane Activity with `Alt+p`, select Deactivate pane, and the remaining
  panes immediately reflow into the smallest column-first layout that fits.
