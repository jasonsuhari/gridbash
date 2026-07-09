# Optimize Runtime Hot Paths

Date: 2026-07-07
Release target: unreleased

## Summary

- Reduced avoidable work in GridBash's runtime loop for busy, high-pane-count sessions.

## What Changed

- Batched pending PTY output by pane before parsing it into vt100 screens.
- Throttled child exit polling to avoid checking every pane on every loop tick.
- Increased the PTY read buffer to reduce message churn during output bursts.
- Preserved these optimizations across active and inactive tab panes.

## Validation

- `cargo check`
