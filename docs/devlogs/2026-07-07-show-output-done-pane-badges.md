# Show output-done pane badges

Date: 2026-07-07
Release target: unreleased

## Summary

- Added subtle output-quiet pane indicators for tracking terminals that have stopped producing output.
- References issues #27 and #29.

## What Changed

- Panes now remember when output last arrived.
- New output clears the quiet state immediately.
- After roughly three seconds without output, a pane becomes `quiet`.
- The pane title now shows a compact `◦` icon for quiet panes.
- Quiet panes use a muted border color instead of a text badge or footer count.

## Why It Matters

- Dense agent grids are easier to scan because panes that need attention stand out after they stop streaming output without adding loud text.
- The first version avoids overclaiming semantic task completion; it tracks output quietness while keeping true process exit distinct.

## Validation

- Ran `cargo fmt --check`.
- Ran `cargo clippy -- -D warnings`.
- Ran `cargo test`.

## Release Notes

- Added subtle quiet-output pane indicators for panes that recently stopped producing terminal output.
