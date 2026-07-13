# Add terminal activity summary

Date: 2026-07-12
Release target: unreleased

## Summary

- Made `Alt+p` show a readable summary of the focused terminal's recent activity.

## What Changed

- Expanded focused-pane settings into a centered activity view that stays readable in dense grids.
- Summarized the pane's latest meaningful output without surfacing raw terminal input.
- Replaced folder, branch, and profile text in pane headers with live activity summaries.
- Made a configured pane goal override the activity summary in that pane's header.
- Preserved pane auth, rename, sleep, and manager-goal controls in the same view.
- Corrected the status-bar labels for `Alt+p` and `Alt+Shift+p`.

## Why It Matters

- A quick shortcut now answers what a terminal has been working on without requiring users to scan its full scrollback.

## Validation

- `cargo fmt --check`
- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `npm run test:launcher`

## Release Notes

- Pane headers now show current activity, or the pane goal when one is set; press `Alt+p` for the focused terminal's detailed output summary and controls.
