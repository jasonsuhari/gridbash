# Fix pane host startup handshake race

Date: 2026-07-20
Release target: unreleased

## Summary

- Fixed session resume failures when terminal persistence is disabled.
- Prevented fresh pane hosts from closing before GridBash can complete the initial handshake.

## What Changed

- Added a bounded startup grace period for a pane host's first client.
- Preserved the existing behavior that terminates non-persistent hosts after their client disconnects.
- Added a regression test that delays the first connection and verifies the pane remains usable.

## Why It Matters

- Saved layouts can now restart cleanly with `keep_terminals_running = false` instead of failing with a reset socket connection.
- Users can disable detached terminals without making new sessions unreliable.

## Validation

- Focused pane-host regression test.
- Rust formatting, linting, and test suite.
- Local install followed by a real resume of the recovered seven-grid layout.

## Release Notes

- Fixed a Windows pane-host handshake failure when resuming sessions with background terminal persistence disabled.
