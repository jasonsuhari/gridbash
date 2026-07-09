# Integrate session resume branch

Date: 2026-07-07
Release target: unreleased

## Summary

- Integrated the older `origin/feat/resume-sessions` work onto latest `origin/main`.

## What Changed

- Resolved session resume against the current app state that includes pane IDs, sleeping panes, worktree labels, usage labels, and runtime resize handling.
- Kept the terminal-query response tail fix so terminal capability/cursor queries do not get repeatedly replayed as input.
- Preserved current modeless pane controls while adding session recorder state, restored pane histories, and `gridbash resume` entry points.

## Why It Matters

- Resume support can land without regressing the recent terminal-grid behavior that current users rely on.

## Validation

- `npm test`

## Release Notes

- Integrate `gridbash resume` with the current GridBash terminal grid.
