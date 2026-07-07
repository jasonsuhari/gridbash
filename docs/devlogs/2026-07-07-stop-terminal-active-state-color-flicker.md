# Stop terminal active-state color flicker

Date: 2026-07-07
Release target: unreleased

## Summary

- Stabilized pane chrome so terminals no longer flash between idle and
  transient output-active styles.

## What Changed

- Removed the short-lived PTY output activity flag from pane border and title
  badge styling.
- Kept stable selected, focused, and exited pane affordances.
- Added regression coverage to keep output activity from changing idle pane
  chrome.

## Why It Matters

- Panes with intermittent output no longer bounce between labels like `main`
  and `main active` or shift border colors during normal use.

## Validation

- `npm test`

## Release Notes

- Fixes distracting terminal pane flicker caused by transient activity badges
  and border color changes.
