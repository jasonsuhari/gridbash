# Hide command line by default

Date: 2026-07-19
Release target: unreleased

## Summary

- Hide the dedicated command line until the user opens it with Alt+C.

## What Changed

- The terminal layout reserves no command-line row while the command line is closed.
- Opening the command line restores its input row and output area, and closing it returns the space to the pane grid.
- Added a focused regression test for the hidden and visible row heights.

## Why It Matters

- Pane grids get an extra row of usable terminal space by default without changing the Alt+C workflow.

## Validation

- `cargo fmt --all -- --check`
- `cargo test command_line_is_hidden_until_focused`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

## Release Notes

- The Alt+C command line is now hidden by default and appears only while open.
