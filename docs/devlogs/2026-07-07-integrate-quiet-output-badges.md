# Integrate quiet output badges

Date: 2026-07-07
Release target: unreleased

## Summary

- Integrated quiet-output pane markers and live palette settings from the older feature branch onto current main.

## What Changed

- Added PTY output quiet tracking that marks a pane after recent output has stopped.
- Added a quiet-output title marker and palette-controlled quiet border while preserving focused, selected, sleeping, exited, usage, and worktree title labels.
- Added live settings rows for accent, focus, selected, quiet, and exited colors without restoring active-state chrome.

## Why It Matters

- Quiet panes are easier to scan in busy grids without confusing idle output for exited processes or reintroducing active-state flicker.

## Validation

- `npm test` passed.

## Release Notes

- Quiet-output markers and runtime palette controls are available in the grid settings screen.
