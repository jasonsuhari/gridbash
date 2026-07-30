# Improve resume picker and stale-client recovery

Date: 2026-07-20
Release target: unreleased

## Summary

- Rebuilt `gridbash resume` as a full-screen, stacked terminal-green picker.
- Made interrupted sessions recoverable without allowing two live clients to attach.
- Hardened pane-host sockets against inherited handles after an unexpected client exit.

## What Changed

- Added a selected-session detail panel above the recent-session list, with concise status badges for open, recoverable, detached, and saved workspaces.
- Added keyboard navigation, pagination, cancellation, and an inline explanation when a session is still open elsewhere.
- Made `resume --latest` skip sessions owned by a live GridBash process when another recoverable session exists.
- Added an optional client PID to the pane-host handshake. Older clients remain compatible, while a new client can replace an attachment whose owning process is gone.
- Marked pane-host listener and client sockets non-inheritable on Windows so spawned shells cannot accidentally keep a dead connection alive.

## Why It Matters

- A terminal or GridBash client can disappear without making its workspace permanently report that it is open elsewhere.
- The resume flow exposes the information needed to choose a workspace without compressing it into a hard-to-scan one-line prompt.

## Validation

- `cargo fmt --all -- --check`
- Focused resume-picker, session, and pane-host tests.
- Full Rust test suite before merge.

## Release Notes

- Fixes [#279](https://github.com/jasonsuhari/gridbash/issues/279).
