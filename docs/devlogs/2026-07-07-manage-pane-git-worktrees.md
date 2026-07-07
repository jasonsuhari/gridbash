# Manage pane git worktrees

Date: 2026-07-07
Release target: unreleased

## Summary

- Added opt-in managed git worktrees for GridBash panes.
- Issue: #45

## What Changed

- Added `--worktrees` to launch each pane in its own repo-local git worktree.
- Added `--worktree-prefix` to customize the managed folder and branch prefix.
- Managed panes create or reuse `.worktrees/<prefix>-<base>-NN` folders with `<prefix>/<base>-pane-NN` branches.
- Pane launch folders preserve the original relative cwd inside each worktree.
- Worktree mode rejects non-git directories and tracked dirty base checkouts with clear errors.

## Why It Matters

- Multiple agents can now work in separate checkouts instead of competing over one working tree.
- Reusing predictable pane branches makes interrupted multi-agent sessions easier to resume.

## Validation

- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo run -- --help`

## Release Notes

- Added `--worktrees` for per-pane git worktree isolation.
