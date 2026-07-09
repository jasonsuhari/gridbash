# Land remaining unmerged branches

Date: 2026-07-09
Release target: unreleased

## Summary

- Audited Codex sessions launched from the GridBash repo, reviewed remaining local and remote branches, and consolidated the useful pending feature work onto one integration branch.
- Ported stale branch work manually where direct merges would have removed newer code, then marked the superseded branch tips as merged so they can be closed and cleaned up.

## What Changed

- Integrated the remaining branch queue into the current app surface: tabbed grids, dedicated command bar, native auth/profile management, hidden manager groups, session resume, idle todo follow-ups, runtime grid resizing, pane rename/swap/deselect/sleep/restart/history settings, worktree management, output idle badges, color palette settings, SEO docs, startup update checks, and release workflow fixes.
- Reviewed local-only branches that were not visible in the remote unmerged check. Their useful patches were already present on this branch, including swap selected tiles, deselect specific panes, contributor onboarding, exact-version release fixes, release retry fixes, and startup grid picker fixes.
- Audited direct user feature requests from GridBash Codex sessions and verified the requested UI/tooling behaviors are present in the integrated tree or represented by tracked docs/devlogs.
- Fixed the one shortcut mismatch found during the audit: `Alt+p` now opens focused-pane settings/history as requested, while the previous-panes list remains available from its status-bar button and `Alt+Shift+p`.

## Why It Matters

- GridBash had useful work spread across stale PRs, local worktrees, and sessions that had not landed on `main`, which made the installed command feel behind the requested feature set.
- Keeping the features on one reviewed integration branch gives `main` a single coherent merge point and makes branch cleanup safer after validation passes.

## Validation

- Pending final integration validation from this branch, then again from local `main` after the PR is merged.

## Release Notes

- Landed the remaining reviewed GridBash feature backlog and aligned the focused-pane settings shortcut with the requested `Alt+p` behavior.
