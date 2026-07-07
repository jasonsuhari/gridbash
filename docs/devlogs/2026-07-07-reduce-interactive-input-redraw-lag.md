# Reduce interactive input redraw lag

Date: 2026-07-07
Release target: unreleased

## Summary

- Reduced interactive typing lag by avoiding unconditional full-frame redraws in the main GridBash loop.

## What Changed

- The main loop now renders only when PTY output, app-control keys, resize events, settings changes, or activity badge decay changes the visible UI.
- Normal terminal input is routed to the focused pane immediately without scheduling an extra stale redraw.
- PTY exit polling and activity decay now report whether they actually changed visible state.

## Why It Matters

- Large grids with multiple active CLI agents can be expensive to repaint. Skipping unchanged frames keeps keyboard input from waiting behind unnecessary full-screen rendering work.

## Validation

- `cargo fmt --check`
- `cargo test`
- `cargo build --release`

## Release Notes

- Fixes input lag during active terminal use by reducing unnecessary redraws.
