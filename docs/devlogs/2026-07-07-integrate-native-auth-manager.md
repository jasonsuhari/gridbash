# Integrate native auth manager

Date: 2026-07-07
Release target: unreleased

## Summary

- Integrated the native Claude/Codex auth profile manager with current `main`.
- Preserved the existing live pane usage labels while making them prefer the auth directory applied to a pane.

## What Changed

- Added `[auth]` config with per-kind defaults and local auth profile discovery.
- Added an Auth tab to Settings for browsing profiles, creating profile directories, setting defaults, refreshing status, and launching login.
- Tagged built-in Claude/Codex profiles with `agent_kind` so auth defaults can set `CLAUDE_CONFIG_DIR` or `CODEX_HOME` at launch.
- Pane titles now include launch profile and auth profile metadata while keeping current usage labels.
- Usage monitoring now follows the applied native auth directory before falling back to existing profile/default auth lookup.

## Why It Matters

- Multiple Claude and Codex accounts can run through GridBash without manually exporting auth environment variables.
- Usage labels stay accurate for panes launched with native auth defaults.
- The Settings/Profile flow from current `main` remains the base behavior.

## Validation

- Ran `cargo fmt`.
- Ran `npm test` with 55 passing tests and 1 ignored interactive ConPTY smoke test after rebasing onto the latest `origin/main`.

## Release Notes

- Added native Claude/Codex auth profile management in Settings.
- Added GridBash-wide auth defaults for built-in and kind-tagged custom agent profiles.
- Pane headers now show launch profile/auth profile metadata alongside existing usage labels.
