# Optimize runtime hot paths

Date: 2026-07-07
Release target: unreleased

## Summary

- Reduced avoidable work in GridBash's runtime loop for busy, high-pane-count
  sessions.

## What Changed

- Batched pending PTY output by pane before parsing it into vt100 screens.
- Added a pane ID index so PTY events no longer scan every pane to find their
  target.
- Throttled child exit polling to avoid checking every pane on every loop tick.
- Cached conversation footer summaries on pane output/resize instead of scanning
  screen rows during every render.
- Added cached rendered screen rows for unchanged panes, with invalidation on
  output, resize, selection, pane swap, and pane removal.
- Increased the PTY read buffer to reduce message churn during output bursts.

## Why It Matters

- Large grids should spend less CPU on quiet panes and repeated metadata work,
  leaving the runtime loop more responsive when one or more panes are noisy.

## Validation

- `cargo test`

## Release Notes

- Improved runtime performance for busy GridBash sessions without changing the
  v1 single-process session model.
