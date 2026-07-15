# Zoom the focused terminal pane

Date: 2026-07-15
Release target: unreleased

## Summary

- Added a reversible focused-pane zoom for dense terminal grids.

## What Changed

- Added `Alt+f` to toggle between the full grid and a single focused pane.
- Preserved zoom independently in every tab without changing pane processes,
  selections, divider weights, or launch metadata.
- Kept hidden pane PTY sizes stable while the visible pane fills the grid.
- Updated in-app help, the README, and the user reference.

## Why It Matters

- A busy agent can temporarily use the whole terminal for review and then
  return to the exact same multi-agent overview.
- Focus navigation remains available while zoomed and predictably retargets
  the full-size view to the newly focused pane.

## Validation

- `cargo fmt -- --check`
- `cargo test zoomed_geometry`
- `cargo test`

## Release Notes

- Press `Alt+f` to zoom the focused terminal pane; press it again to restore
  the grid.
