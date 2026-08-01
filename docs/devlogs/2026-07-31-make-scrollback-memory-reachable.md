# Let scrollback be turned down far enough to matter

Date: 2026-07-31
Release target: unreleased

## Summary

- Scrollback is the largest thing a running grid holds, and the setting could
  not be turned below a thousand rows per pane. The floor is now two hundred.

## What Changed

- `MIN_SCROLLBACK_ROWS`, `MAX_SCROLLBACK_ROWS`, and `clamp_scrollback_rows` live
  in one place, and every path that accepts a scrollback figure uses them: the
  config file, the settings panel, the panel's save-failure rollback, and the
  parser a pane is built with. There were five separately written copies of the
  same `clamp(1_000, 50_000)`.
- The settings stepper moves in two hundreds below a thousand rows and in
  thousands above it, so the low end can be reached at all.
- The agent control token is compared without letting the comparison's duration
  report how much of the token was correct.

## Why It Matters

- A terminal parser holds a thirty-two byte cell for every column of every
  retained row, whether or not anything was written there. Measured, not
  estimated: `size_of::<vt100::Cell>()` is 32, and twenty filled parsers at two
  hundred columns occupy a working set matching the arithmetic to within a
  megabyte.

  | scrollback | per pane | twenty panes |
  | ---------- | -------- | ------------ |
  | 200        | 1.5 MB   | 30 MB        |
  | 1000 (old floor) | 6.4 MB | 128 MB  |
  | 3000       | 18.6 MB  | 372 MB       |
  | 10000 (default) | 61.3 MB | 1226 MB |

  The default is over a gigabyte of scrollback for a twenty-pane workspace, and
  until now a user who noticed could only take it down to a hundred and
  twenty-eight megabytes. Panes that genuinely need history can be logged to
  disk, which costs nothing resident.

## Validation

- `cargo test --bin gridbash -- --test-threads=1`: 397 passed, 5 ignored.
- `cargo clippy --bin gridbash --all-targets -- -D warnings` and
  `cargo fmt --check` are clean.
- New tests cover stepping the setting to both ends of its range and the shared
  clamp, and confirm a token is accepted only on an exact match.

## Release Notes

- Scrollback can be set as low as two hundred rows per pane.
- The default remains ten thousand; lowering it is the single largest memory
  saving available to a large grid.
