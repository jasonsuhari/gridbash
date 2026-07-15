# Optimize high-load runtime throughput and latency

Date: 2026-07-15
Release target: unreleased

## Summary

- Removed more avoidable CPU and allocation work from GridBash's busy-grid
  event, output-processing, and redraw paths.

## What Changed

- Cached fully rendered pane cell buffers instead of cloning and rebuilding
  styled terminal lines for every unchanged pane on every frame.
- Skipped idle PTY routing maps and per-pane workload scans until an event or
  visible state change actually requires them.
- Kept PTY output batches in arrival order while replacing tree-based batching
  with constant-time hash lookups.
- Added an ASCII-run fast path for terminal history text, tracked its character
  count during filtering, and avoided duplicate control-sequence allocations.
- Replaced sleeping-pane text allocation with direct frame-buffer clearing.
- Added correctness coverage and opt-in release-mode microbenchmarks for the
  optimized text and screen-cache paths.

## Why It Matters

- Large grids spend most of their UI time revisiting panes that did not change.
  Reusing final cell buffers and avoiding idle whole-grid scans leaves more CPU
  for the agents and reduces input latency while output is flowing.
- In the release-mode probes, unchanged 120x40 pane rendering improved from
  273.6 to 75.3 microseconds per frame (3.6x faster), and ANSI-to-history text
  filtering improved from 26.1 to 14.4 microseconds (45% faster).

## Validation

- `cargo fmt --all -- --check`
- `cargo check --tests`
- `cargo test --release` (171 passed, 3 intentionally ignored)
- `cargo clippy --all-targets -- -D warnings`
- `npm run test:launcher` (11 passed)
- `cargo build --release`
- `cargo test --release benchmark_ -- --ignored --nocapture --test-threads=1`

## Release Notes

- GridBash now uses substantially less work to redraw unchanged terminal panes
  and process sustained agent output, especially in large, busy grids.
