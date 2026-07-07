# Show output-done pane badges

Date: 2026-07-07
Release target: unreleased

## Summary

- Added output-quiet pane badges for tracking terminals that have stopped producing output.
- References issue #27.

## What Changed

- Panes now remember when output last arrived.
- New output clears the quiet state immediately.
- After roughly three seconds without output, a pane becomes `quiet`.
- The pane title and footer count now expose quiet panes separately from active and exited panes.

## Why It Matters

- Dense agent grids are easier to scan because panes that need attention stand out after they stop streaming output.
- The first version avoids overclaiming semantic task completion; it tracks output quietness while keeping true process exit distinct.

## Validation

- Ran `cargo fmt --check`.
- Ran `cargo clippy -- -D warnings`.
- Ran `cargo test`.

## Release Notes

- Added quiet-output pane badges and a footer count for panes that recently stopped producing terminal output.
