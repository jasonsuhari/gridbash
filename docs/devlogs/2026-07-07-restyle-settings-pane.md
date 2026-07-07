# Restyle settings pane

Date: 2026-07-07
Release target: unreleased

## Summary

- Restyled the settings pane into a more polished terminal modal with clearer grouping and hierarchy.

## What Changed

- Added a shadowed settings modal frame with a stronger GridBash Settings title treatment.
- Grouped settings rows into Display, Workflow, Performance, and Theme sections.
- Reworked selected rows, value pills, command hints, and row helper text for better scanning.
- Added width-aware truncation and shorter command text for narrow terminal layouts.

## Why It Matters

- The settings pane now feels like a deliberate product surface instead of a placeholder list.
- Users can scan the available controls faster and see the selected row more clearly.

## Validation

- Ran `cargo test`.
- Result: 13 passed, 0 failed, 1 ignored.

## Release Notes

- Restyled the settings pane with clearer sections, selected-state polish, and responsive terminal text.
