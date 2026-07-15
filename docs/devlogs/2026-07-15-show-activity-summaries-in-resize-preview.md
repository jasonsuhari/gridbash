# Show activity summaries in resize preview

Date: 2026-07-15
Release target: unreleased

## Summary

- Added current pane activity summaries to the `Alt+l` resize preview.

## What Changed

- Captured available activity summaries when the resize picker opens.
- Rendered summaries inside their matching blue preview cells while leaving panes without a summary unlabeled.
- Preserved row-and-column mappings as the proposed grid dimensions change, so retained panes keep the correct summary.

## Why It Matters

- Pane content is now identifiable before applying a resize, making it clearer which panes will be retained or removed.

## Validation

- `cargo test composer::tests`
- `cargo test`
- `cargo fmt -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `git diff --check`

## Release Notes

- The `Alt+l` grid resize preview now shows available activity summaries inside existing pane cells.
