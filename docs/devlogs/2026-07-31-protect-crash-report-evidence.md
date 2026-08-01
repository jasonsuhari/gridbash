# Stop routine failures from evicting crash reports

Date: 2026-07-31
Release target: unreleased

## Summary

- The crash report directory holds a fixed number of reports, and almost
  everything in it was something other than a crash.

## What Changed

- `GRIDBASH_LOGS_DIR` overrides the report directory, and a test binary
  redirects itself to a temporary one instead of writing to the real directory.
- Each entry point reports under its own role rather than all reporting as the
  interface.
- One-shot control commands no longer record ordinary usage errors as error
  exits. A panic in them is still recorded.
- Pruning drops routine reports before panic reports.

## Why It Matters

- The worker recovery test records a recovered panic every time it runs, and
  forty three of those had collected in a directory that holds fifty. A real
  panic report was a few test runs away from being pruned, and the panic that
  was investigated this week nearly was.
- A control command naming a pane that no longer exists wrote an error-exit
  report filed as a crash of a TUI that was never running. Agent panes address
  each other by pane number, so a stale one quietly evicted real reports.
- A panic report is the only record that survives the process. Everything else
  in the directory can be reconstructed from somewhere, so nothing else should
  be able to push a panic out.

## Validation

- `cargo test --bin gridbash -- --test-threads=1`: 392 passed, 5 ignored.
- `cargo clippy --bin gridbash --all-targets -- -D warnings` and
  `cargo fmt --check` are clean.
- Confirmed against the real directory: a full test run left its file count and
  newest timestamp unchanged, and the reports it produced landed in the
  temporary directory instead.

## Release Notes

- Crash reports are no longer crowded out by recovered failures, control
  command usage errors, or test runs.
