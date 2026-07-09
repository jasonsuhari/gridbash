# Add previous panes list button

Date: 2026-07-09
Release target: unreleased

## Summary

- Added a status-bar Panes button that opens a Codex-style list of current pane conversations.

## What Changed

- Added `Alt+p` and a clickable `Panes` status-bar control.
- Added a Previous Panes modal with pane labels, state, folder/worktree context, and the latest visible conversation summary.
- Let users move through the list with arrows and focus a pane with Enter, Space, or a row click.

## Why It Matters

- Dense GridBash sessions are easier to navigate because users can jump by conversation context instead of scanning every pane manually.

## Validation

- `cargo fmt --check`
- `cargo test`
- `cargo clippy -- -D warnings`

## Release Notes

- Added a previous panes list button and `Alt+p` selector for focusing panes by their conversation summary.
