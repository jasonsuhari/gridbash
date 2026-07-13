# Expand command CLI with Alt+C

Date: 2026-07-13
Release target: unreleased

## Summary

- Consolidated command-line focus and captured-output visibility under Alt+C.

## What Changed

- Alt+C now expands and focuses the command line, then closes it on the next press.
- Removed the separate Alt+E command-output shortcut and updated the shortcut legends.

## Why It Matters

- Command output is visible as soon as users enter the command line, without a second shortcut.

## Validation

- Added a focused unit test for command-line focus and output visibility.
- Ran Rust formatting, checks, and tests.

## Release Notes

- Alt+C now opens the full command-line view, including captured output.
