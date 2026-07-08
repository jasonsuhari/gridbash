# Agent control tools

Date: 2026-07-07
Release target: unreleased

## Summary

- Added an opt-in GridBash agent control surface for MCP-capable panes.

## What Changed

- Added `--agent-api` to start a localhost-only control server with a per-session token.
- Added `--mcp` to run a stdio MCP server exposing `gridbash_show_image`, `gridbash_send_command`, and `gridbash_set_status`.
- Injected control endpoint, token, and pane index environment variables into child panes when the agent API is enabled.
- Added an in-app image overlay that renders local images as truecolor terminal cells.
- Documented the agent control setup in the README.

## Why It Matters

- Manager agents can now send targeted commands to other panes and update the GridBash status line without broad UI automation.
- Agents can surface local image artifacts directly in GridBash for quick visual review.
- The control path remains disabled by default and scoped to the current local session.

## Validation

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- MCP stdio smoke test for `initialize` and `tools/list`.

## Release Notes

- Added opt-in MCP tools for showing images, sending pane commands, and setting the GridBash status line.
