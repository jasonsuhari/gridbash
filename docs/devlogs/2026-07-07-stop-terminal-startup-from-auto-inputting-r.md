# Stop terminal startup from auto-inputting R

Date: 2026-07-07
Release target: unreleased

## Summary

- Fixed a PTY terminal-query scanner issue tracked in #18 where startup could leave a stray `R` typed into a newly launched pane.

## What Changed

- Retain only partial ANSI terminal-query prefixes between PTY output chunks.
- Added coverage for split cursor-position queries and complete-query replay prevention.

## Why It Matters

- Startup shells can ask for cursor position with `ESC[6n`; GridBash should answer that query once, not replay the answer later as pane input.

## Validation

- Ran `cargo fmt`.
- Ran `cargo test`.

## Release Notes

- Fixed terminal startup sometimes auto-inputting `R` into panes.
