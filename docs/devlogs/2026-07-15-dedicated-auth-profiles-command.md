# dedicated auth profiles command

Date: 2026-07-15
Release target: unreleased

Issue: [#232](https://github.com/jasonsuhari/gridbash/issues/232)

## Summary

- Added a dedicated Auth Profiles command that makes managed accounts, focused-pane assignment, and new-pane launch policy understandable in one place.

## What Changed

- Added Alt+Shift+A as a direct Auth Profiles shortcut while preserving Alt+A for select-all.
- Reworked the Auth view around three explicit concepts: the focused pane's current account, the policy for future panes, and the isolated profile list.
- Added Enter-to-assign for the highlighted compatible profile. Assignment reuses the existing safe pane restart/session-save path and leaves other panes running.
- Renamed user-facing auto-cycle copy to round-robin and clarified that defaults and round-robin only affect panes when they start.
- Updated in-app help, status guidance, README controls, and the auth reference.

## Why It Matters

- Managed auth previously existed across global Settings and Pane Activity, but the entry point and scope of each control were easy to miss.
- The dedicated view now explains that profiles are isolated Claude/Codex homes and shows the restart consequence before a current pane is switched.

## Validation

- `cargo test auth -- --test-threads=1` (15 passed)
- `cargo test` with `GRIDBASH_INVOKING_PROFILE` removed from the child test environment (171 passed, 1 ignored after merging current `main`)
- `cargo clippy --all-targets -- -D warnings`
- `npm test` with `GRIDBASH_INVOKING_PROFILE` removed from the child test environment (171 passed, 1 ignored after merging current `main`)

## Release Notes

- Press Alt+Shift+A to manage isolated Claude/Codex auth profiles, assign one to the focused pane, or choose defaults versus round-robin assignment for new panes.
