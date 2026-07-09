# Wrap Alt arrow pane navigation

Date: 2026-07-09
Release target: unreleased

## Summary

- Alt-arrow pane navigation now wraps at row and column edges instead of stopping or crossing into the next row.

## What Changed

- Changed horizontal focus movement to stay within the current row and wrap from the right edge to the left edge, and vice versa.
- Changed vertical focus movement to stay within the current column and wrap from the bottom edge to the top edge, and vice versa.
- Kept sleeping panes out of focus traversal when wrapping.

## Why It Matters

- Pane movement now behaves predictably in grid space, so repeated Alt-arrow presses cycle through the visible row or column the user is already navigating.

## Validation

- `cargo fmt --check`
- `cargo test focus`
- `cargo test`

## Release Notes

- Alt-arrow pane focus now wraps within the current row or column at grid edges.
