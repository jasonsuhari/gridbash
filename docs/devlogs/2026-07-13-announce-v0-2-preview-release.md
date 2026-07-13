# Announce v0.2 preview release

Date: 2026-07-13
Issue: #209
Release target: v0.2.0-macos.1

## Summary

- Add a release-specific announcement kit for the v0.2 cross-platform preview.
- Lead with the new coordination model instead of repeating the original
  Windows-only launch copy.

## What Changed

- Added short X/Bluesky copy, a launch reply, a longer GitHub/LinkedIn post,
  and a compact Discord version.
- Highlighted grid-wide manager orchestration, per-pane Codex SQLite lanes,
  five native build targets, and nightly releases.
- Added explicit publication guardrails so the copy links the live GitHub
  prerelease without claiming npm availability before registry bootstrap.

## Why It Matters

- The announcement now sells the real v0.2 story: GridBash is becoming a
  command center for a squad of agents, not just a way to tile terminals.
- Platform and installation claims match artifacts that actually exist.

## Validation

- Confirmed `v0.2.0-macos.1` is a live GitHub prerelease with exactly six
  tarballs for the launcher and five native targets.
- Confirmed the forced nightly targets integrated `main` and also has six
  GitHub assets.
- Kept the npm install line disabled until issue #192's one-time authentication
  bootstrap publishes all six packages.

## Release Notes

- Announcement kit: `docs/V0_2_ANNOUNCEMENT.md`
- Tracking: #209
