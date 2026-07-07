# Default multi-select input

Date: 2026-07-07
Release target: unreleased

## Summary

- Removed the explicit selected-input mode toggle.
- Input now targets selected panes automatically when more than one pane is selected.

## What Changed

- Removed the Alt+b mode switch from the app controls.
- Updated the status bar to show whether input is headed to the focused pane or selected panes.
- Updated docs to describe automatic multi-pane input.

## Why It Matters

- Multi-pane input now follows visible pane selection directly, so there is no extra mode to discover or remember.
- A single selected pane remains visual selection only; input still follows the focused pane until multiple panes are selected.

## Validation

- Added app-level regression tests for focused-pane and selected-pane input targeting.
- Ran `cargo test`.

## Release Notes

- Selecting multiple panes now sends input to those panes by default. The old Alt+b toggle has been removed.
