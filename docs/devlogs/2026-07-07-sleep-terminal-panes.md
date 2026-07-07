# Sleep terminal panes

Date: 2026-07-07
Release target: unreleased

## Summary

- Add pane sleep mode for hiding visually noisy terminal output until the pane is needed again.

## What Changed

- Added `Alt+z` to sleep the focused pane, or the selected panes when multiple panes are selected.
- Sleeping panes render as black terminal interiors while their PTY processes and output history keep running.
- Hovering a sleeping pane wakes it and focuses it.
- Mouse capture is enabled only while panes are asleep, then released after the final sleeping pane wakes.

## Why It Matters

- Dense agent grids can stay running without every active terminal competing for visual attention.

## Validation

- `cargo test` with `CARGO_TARGET_DIR=C:\Users\Jason\Documents\GitHub\gridbash\target`

## Release Notes

- New terminal pane sleep mode: press `Alt+z` to black out a pane, then hover it to wake it.
