# Native auth profile manager

Date: 2026-07-07
Release target: unreleased

## Summary

- Added a native Claude/Codex auth profile manager to GridBash Settings.
- GridBash can now apply global per-kind auth defaults when launching Claude or Codex panes.

## What Changed

- Added local auth profile discovery for `~/.claude-profiles`, with `GRIDBASH_AUTH_HOME`, `CLAUDE_PROFILES_HOME`, and config overrides.
- Added `[auth]` config with Claude/Codex defaults and best-effort usage status.
- Added an Auth tab in Settings for browsing profiles, creating profile directories, setting defaults, refreshing status, and launching login.
- Built-in Claude/Codex profiles and custom profiles with `agent_kind` now receive `CLAUDE_CONFIG_DIR` or `CODEX_HOME` when a default is configured.
- Pane titles show the auth profile name when one is applied.

## Why It Matters

- Multiple Claude and Codex accounts can run in different GridBash sessions without manually exporting environment variables.
- Auth defaults are GridBash-wide for now, keeping the first implementation simple while leaving room for per-pane auth later.
- Usage/account metadata is loaded in the background from Settings so normal pane rendering and input stay responsive.

## Validation

- Ran `cargo fmt`.
- Ran `cargo test` with 19 passing tests and 1 ignored interactive ConPTY smoke test.
- Ran `cargo run -- --list-profiles` and confirmed the profile list still renders with Claude/Codex available on this machine.

## Release Notes

- Added native Claude/Codex auth profile management in Settings.
- Added global auth defaults for built-in and kind-tagged custom agent profiles.
- Added config and README documentation for isolated auth profile directories.
