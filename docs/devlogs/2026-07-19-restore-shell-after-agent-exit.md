# Restore shell after agent exit

Date: 2026-07-19
Release target: unreleased

## Summary

- Restore an interactive shell in a pane after a directly launched coding agent exits.
- Closes #274.

## What Changed

- Agent exit events now replace the exited process with the preferred available terminal profile.
- The replacement shell keeps the pane's current working directory, visible output, input history, and pane metadata.
- Inactive tabs recover exited agent panes when the tab becomes active.
- Non-agent exits continue to use the existing exited-pane recovery dialog.

## Why It Matters

- Leaving Codex with `/exit` no longer leaves the pane unusable or requires a manual restart.
- The pane remains useful as a normal terminal without losing its working context.

## Validation

- `cargo fmt --all -- --check`
- `cargo test exited_`

## Release Notes

- Fixed agent `/exit` leaving GridBash panes in the exited state instead of returning them to a shell.
