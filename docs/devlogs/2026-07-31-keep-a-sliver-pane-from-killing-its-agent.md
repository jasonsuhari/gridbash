# Keep a sliver pane from killing its agent on resume

Date: 2026-07-31
Release target: unreleased

## Summary

- A restored pane too narrow to hold a terminal crashed its pane host the moment
  an agent drew a wide character, killing the agent and leaving a bare shell
  wearing the dead agent's scrollback.
- One Claude conversation could also be saved against two panes, which resumed
  it twice and left the losing pane in the same state.

## What Changed

- Clamp every pane to at least two rows and two columns, in the view, the PTY,
  and the pane-host wire protocol, so the three never disagree about the size a
  parser is holding.
- Resolve Claude conversation ownership when a workspace is saved: a pane that
  pinned its conversation at launch owns it, a pane that only mentions one in
  its typed history yields, and the first claimant keeps it.
- Apply the same rule when a snapshot is loaded, because snapshots written
  before this change already name one conversation on two panes.

## Why It Matters

- vt100 cannot place a double-width character in a one-row or one-column grid:
  `col_wrap` computes `cols - width` as `1 - 2`, underflows, and then panics on
  the cell that should hold the character's second half. Panes were clamped to
  `1x1`, so any emoji or CJK character in replayed output took the host down.
  Restored grids reach sliver widths far more often than live ones, which is why
  this surfaced as a resume failure.
- Resuming one conversation twice starts a second Claude that exits immediately,
  so a pane that looked recovered was a plain shell showing replayed text.

## Validation

- `cargo test --bin gridbash -- --test-threads=1`: 388 passed, 5 ignored.
- `cargo clippy --bin gridbash --all-targets -- -D warnings` and
  `cargo fmt --check` are clean.
- `tiny_panes_survive_wide_characters` was confirmed to fail with the clamp
  lowered to one row and column, reproducing the reported panic, and to pass
  with it restored.
- A standalone sweep over vt100 established two rows by two columns as the
  smallest safe grid: 22,000 mixed narrow, CJK, and emoji inputs at or above
  that size produced no panic, and every size below it did.

## Release Notes

- Panes restored into a very small cell no longer take their agent down.
- A Claude conversation is never resumed into two panes at once.
