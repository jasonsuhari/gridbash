# Make pane-to-pane prompting discoverable to agents

Date: 2026-07-27
Release target: unreleased

## Summary

- Added an agent-oriented pane discovery and prompting interface for manager
  panes and delegation workflows.

## What Changed

- Added `gridbash agent panes` with caller-aware output and stable pane targets.
- Added `gridbash agent prompt` with positional or piped stdin input, repeated
  explicit targets, `--others`, and optional no-submit behavior.
- Added the `gridbash_prompt_panes` MCP tool with stable targets and a
  caller-excluding other-panes mode.
- Enabled local authenticated pane tools for fresh sessions by default, with
  `--no-agent-api` as an explicit opt-out.
- Exposed a concise `GRIDBASH_AGENT_TOOLS` discovery hint inside child panes.

## Why It Matters

- A coding agent can now recognize the purpose-built GridBash commands from CLI
  help or its pane environment, then coordinate siblings without manually
  copying session IDs or bearer tokens.
- One pane can act as the manager and safely prompt every other available pane
  without accidentally prompting itself.

## Validation

- Pending focused Rust tests, formatting, and the repository validation batch.

## Release Notes

- From any GridBash pane, run `gridbash agent panes`, then prompt a target with
  `gridbash agent prompt --pane <stable-target> "..."`.
- Pipe a generated instruction to every other available pane with
  `... | gridbash agent prompt --others`.
