# Fix terminal selection containment

Date: 2026-07-07
Release target: unreleased

## Summary

- Fixed mouse text selection leaking across terminal panes by making drag
  selection pane-contained.

## What Changed

- GridBash now captures mouse drag selection by default and clamps the active
  selection to the pane where the drag began.
- Selected terminal cells are highlighted only in the source pane.
- Releasing the drag copies the selected pane text via the OSC 52 terminal
  clipboard sequence.
- The hidden `--no-mouse` compatibility flag still leaves selection to the host
  terminal when needed.

## Why It Matters

- Selecting text in one pane no longer drags a visual selection through sibling
  panes or interferes with their input/focus state.

## Validation

- `cargo test`
- `cargo clippy -- -D warnings`

## Release Notes

- Fixes pane text selection so drag selection stays contained to the terminal
  pane where it starts.
