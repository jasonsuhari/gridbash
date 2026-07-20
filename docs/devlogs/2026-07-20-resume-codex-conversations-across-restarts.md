# Resume Codex conversations across restarts

Date: 2026-07-20
Release target: unreleased

## Summary

- GridBash now remembers the active Codex conversation behind each saved pane
  and resumes it when the original terminal host no longer exists.

## What Changed

- Persist the active Codex thread ID alongside pane history and host metadata.
- Detect Codex descendants launched through Git Bash as well as direct Codex
  profiles.
- Rebuild unavailable panes as `codex resume <conversation-id>` without
  replaying unrelated shell history.
- Keep older session snapshots compatible through optional serialized fields.

## Why It Matters

- Layout snapshots already survived `Alt+q`, but the Codex process itself could
  not survive a laptop shutdown. Reconnecting to the conversation restores the
  useful agent context instead of leaving seven empty terminals.

## Validation

- Focused Rust tests cover thread lookup, process-tree ownership, direct Codex
  resume commands, Git Bash conversion, and existing resume wrappers.
- Full validation is recorded on the pull request for this change.

## Release Notes

- Fixes #295.
