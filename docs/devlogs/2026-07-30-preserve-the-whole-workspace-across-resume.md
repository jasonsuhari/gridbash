# Preserve the whole workspace across resume

Date: 2026-07-30
Release target: unreleased

## Summary

- Resuming a workspace now rebuilds it as it was: grid sizes, pane positions,
  grid and pane names, focus, zoom, dragged dividers, tab order, and each pane's
  agent conversation.
- Claude panes resume their conversation instead of coming back as bare shells.
- Fixes #304.

## What Changed

- Snapshot format v2 records pane names, Claude session ids, sleeping panes,
  per-grid view state (focus, zoom, divider weights), the active grid's position
  in the tab strip, and the next grid number. Version 1 snapshots still load with
  the new fields defaulted.
- Saved grid dimensions are used verbatim. A 2x3 grid holding four panes comes
  back 2x3 with two empty cells, and panes are placed by the cell they recorded
  rather than by their order in the file.
- Crash recovery carries each interrupted workspace's grids over unchanged.
  It previously pooled panes by working directory and re-derived a grid size from
  the pane count, which reshaped a 2x3 grid into 3x3 and replaced grid names with
  folder names. Recovery now also keeps background panes and opens on the grid
  that was interrupted.
- Claude panes are pinned to a conversation at launch with `--session-id`, so a
  snapshot names it exactly even with several Claude panes in one folder.
  Restores use `claude --resume` after checking the transcript still exists, and
  fall back to a plain launch when it does not. Panes are re-checked periodically
  so one that clears its conversation follows onto the new one.
- Snapshots are written durably: contents are flushed to disk before the rename
  publishes them, the previous snapshot is kept as a `.bak`, and loading falls
  back to it when the primary copy is unreadable.
- Changes to a workspace's shape are saved immediately rather than waiting for
  the autosave tick, and a drop guard saves state that a panic outside the event
  loop's firewall would otherwise take with it.
- The resume picker lists every grid with the size it will be rebuilt at, and
  `Tab` walks a positional map showing each pane in its cell with the name it was
  given and a mark for conversations that resume.

## Why It Matters

- A crash used to cost the arrangement even when the panes came back: grids were
  the wrong size, grids and panes lost their names, and Claude conversations were
  gone while the restored scrollback made it look like they had survived.

## Validation

- `cargo test` (371 passed), `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check`, and a release build.
- New regression tests cover grid-dimension preservation, cell placement, tab
  order and the active grid, recovery fidelity, background-pane retention, Claude
  pinning/resume/fallback/follow-on, version 1 migration, and backup fallback.
- `gridbash resume --list` read the existing version 1 snapshots on this machine
  from both the debug and release binaries.

## Release Notes

- Resumed and recovered workspaces keep their grid sizes, pane positions, grid
  and pane names, focus, zoom, and tab order.
- Claude panes resume their conversation, including one started after the pane
  launched.
- Session snapshots survive an interrupted write.
