# Define v1 and daemon architecture

Date: 2026-07-12
Release target: unreleased

## Summary

- Added concrete stable-v1 acceptance gates and the proposed detach/reattach architecture.

## What Changed

- Defined release evidence for installation, PTY lifecycle, profiles, interaction,
  sessions, worktrees, documentation, packaging, and stability decisions.
- Defined daemon/client ownership, local IPC trust, synchronization, multi-client
  leases, persistence, compatibility, failure behavior, required prototypes, and
  open design decisions.
- Linked both documents from the README and roadmap.

## Why It Matters

- Stable-release and daemon work now have reviewable boundaries instead of broad
  milestone labels that could hide incompatible assumptions.

## Validation

- `git diff --check`
- Reviewed links and headings against the current README, roadmap, CI, packaging,
  profile, PTY, session, and release behavior.

## Release Notes

- Documented the GridBash v1.0 release gates and post-v1 detach/reattach design.
