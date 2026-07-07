# Fix redraw lag clippy

Date: 2026-07-07
Release target: unreleased

## Summary

- Fixed the clippy failure introduced with the interactive redraw-lag change.
- Tracked the main-branch CI failure through GitHub issue #26.

## What Changed

- Collapsed the nested pane-exit condition in `src/app.rs`.
- Kept the behavior unchanged: only matching pane generations that are not already exited set `changed = true`.

## Why It Matters

- `main` needs green CI before the `0.1.4` release workflow can publish safely.
- The release should include the redraw-lag fix from PR #23, but not while main validation is failing.

## Validation

- Ran Rust formatting, clippy, and tests.
- Ran whitespace checks.

## Release Notes

- Fixed CI validation for the redraw-lag pane exit handling change.
