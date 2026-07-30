# Fix torn pane rendering when switching tabs

Date: 2026-07-20
Release target: unreleased

## Summary

- Fixed torn or partially blank terminal content after switching tabs with
  different grid geometries.

## What Changed

- Recomputed pane rectangles from the currently visible grid immediately after
  restoring a tab.
- Resized restored PTYs before the switched tab's first rendered frame.
- Reused the same layout synchronization helper during initial pane sizing.

## Why It Matters

- Dense grids use smaller PTY dimensions than tabs with a few large panes.
  Rendering a restored pane before resizing it could mix the old screen bounds
  with the new rectangle, leaving content scrambled until another full redraw.

## Validation

- `cargo fmt --all -- --check`
- `cargo check`

## Release Notes

- Switching grids with `Alt+t` now keeps terminal content intact when the tabs
  use different layouts.

Closes #282.
