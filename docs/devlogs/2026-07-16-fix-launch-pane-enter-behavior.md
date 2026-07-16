# Fix launch pane Enter behavior

Date: 2026-07-16
Release target: unreleased

## Summary

- Made the startup workspace launch action explicit and reliable across terminal Return encodings.

## What Changed

- Added a selectable Launch row after the workspace configuration fields.
- Kept Enter as a global launch action from every composer row.
- Normalized Enter, carriage return, newline, and Ctrl+M while preserving project-folder editing.
- Added focused regression coverage for every composer field and Return aliases.

## Why It Matters

- Users can launch the configured workspace without guessing whether a highlighted setting is blocking Enter.
- Terminals that report Return as CR or Ctrl+M now trigger the same action as a normal Enter event.

## Validation

- `cargo fmt --all -- --check`
- `git diff --check`
- Cross-platform Cargo validation runs in pull-request CI.

## Release Notes

- Fixed the startup workspace composer so Enter reliably launches and Launch appears as a selectable row.
