# Keep terminals running after exit

Date: 2026-07-15
Release target: unreleased

## Summary

- Added an opt-in way to close GridBash while its live terminal processes keep
  running locally.

## What Changed

- Moved pane PTY ownership into authenticated local background hosts.
- Added a **Keep terminals running** workflow setting that updates active and
  inactive-tab panes immediately.
- Saved pane-host references with session snapshots so `gridbash resume`
  reconnects to the same PTYs and receives output produced while detached.
- Kept the existing stop-on-close behavior as the default and fall back to a new
  terminal with saved context if a background host is unavailable.

## Why It Matters

- Long-running agents, builds, and shells no longer have to stop just because a
  user closes the GridBash interface.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --no-fail-fast`
- `npm test`
- Manual detach/reattach lifecycle coverage through the pane-host integration
  test.

## Release Notes

- GridBash can now keep live terminals running after the UI closes and reattach
  to them from a saved session.
