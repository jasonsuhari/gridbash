# Recover interrupted conversations into tabs

Date: 2026-07-18
Release target: unreleased

## Summary

- Recover agent conversations left behind when GridBash or its host terminal
  closes unexpectedly.

## What Changed

- Added running-owner and clean-exit metadata to backward-compatible session
  snapshots.
- Autosave active grids every five seconds so an abrupt close retains recent
  pane input, output, host references, tabs, and background jobs.
- On a plain launch, atomically claim sessions whose owner process is gone,
  group every saved pane by working directory, and reopen the groups as
  directory-named tabs.
- Skip sessions owned by live GridBash processes and keep explicit launch
  arguments and `gridbash resume` behavior unchanged.
- Added an `Alt+t switch` hint directly to the tab bar.

## Why It Matters

- Interrupted multi-project work returns as one coherent workspace without
  making people remember session IDs or rebuild grids by hand.

## Validation

- `cargo fmt --all -- --check`
- `cargo test session::tests`
- `cargo test cli::tests`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `npm run install:local` from merged `main`

## Release Notes

- Automatically recover interrupted agent sessions into directory-named tabs;
  press `Alt+t` to switch between them.
