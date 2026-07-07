# Idle terminal todo prompts

Date: 2026-07-07
Release target: unreleased

## Summary

- Added settings-managed todo prompts that can be offered to idle terminals.

## What Changed

- Added persisted `[todos]` config support for enabling idle prompts, setting the quiet delay, and storing prompt text.
- Added a TODO section to the redesigned settings pane with add, edit, delete, enable, and quiet-delay controls.
- Added per-pane quiet tracking and a small follow-up dialog that can send, cycle, remove, or dismiss queued prompts.
- Updated the example config with sample todo prompts.

## Why It Matters

- Users can keep spare terminals productive by preparing follow-up prompts ahead of time, then sending one when a pane has gone quiet.

## Validation

- `cargo check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

## Release Notes

- New settings todo list can suggest queued follow-up prompts for quiet panes.
