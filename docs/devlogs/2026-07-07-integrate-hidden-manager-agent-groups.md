# Integrate hidden manager agent groups

Date: 2026-07-07
Release target: unreleased

## Summary

- Integrated hidden manager agent groups from `origin/feat/hidden-agent-groups` onto current `main` without carrying over the old branch's unrelated deletes.
- Tracks issue #66.

## What Changed

- Added `--manager-profile`, `GRIDBASH_MANAGER_PROFILE`, and `[defaults].manager_profile` resolution for hidden group managers.
- Added hidden manager PTYs that coordinate selected awake worker panes, parse manager `gridbash send` blocks, and relay worker output snapshots back to managers.
- Added group badges and a manager prompt overlay while preserving visible pane arrays for resize, sleep, swap, worktree labels, usage labels, and pane-contained input selection.
- Added focused tests for manager config parsing, Vibe profile resolution, send-block parsing, and existing pane behavior.

## Why It Matters

- Manager agents can coordinate worker panes without consuming a visible grid slot.
- The integration avoids regressing current `main` behavior that landed after the source feature branch diverged.

## Validation

- `npm test`
- Result: 34 passed, 0 failed, 1 ignored Windows ConPTY smoke test.

## Release Notes

- Added hidden manager agent groups behind `Alt+g`/`Alt+u` and manager profile configuration.
