# Reposition GridBash as an Agent Workspace

Date: 2026-07-15
Issue: #239
Release target: unreleased

## Summary

- Repositioned GridBash as a local workspace for running and coordinating CLI
  coding agents in parallel.

## What Changed

- Replaced first-run terminal onboarding and the grid-only startup picker with
  one agent-workspace setup for profile, auth, project, layout, and worktrees.
- Listed detected agents before clearly labeled raw-terminal profiles while
  preserving explicit CLI launches and normal machine-wide agent commands.
- Updated product, package, website, launch-kit, configuration, and reference
  copy around the agent-workspace category.

## Why It Matters

- Managed auth now has an honest product boundary: it applies to agents
  GridBash launches, while ordinary terminals and commands remain untouched.
- Users can still open raw terminal grids without making GridBash compete with
  their everyday shell or general-purpose terminal multiplexer.

## Validation

- `cargo fmt --all -- --check`
- `git diff --check`
- Parsed `package.json` with Node.js.
- Compiler, test, and installer validation are delegated to the automatic
  combined integration batch.

## Release Notes

- GridBash now opens as a managed local agent workspace, with raw terminal grids
  retained as an explicit secondary option.
