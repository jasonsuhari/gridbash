# v0.2 cross-platform preview

Date: 2026-07-13
Issue: #192
Release target: v0.2.0-macos.1

## Summary

- Aggregate the product, platform, and release work on `main` since `v0.1.6`
  into one cross-platform preview.
- Keep the release a prerelease while the macOS signing gate is open.
- Manager-grid orchestration (#187) and per-pane Codex SQLite isolation (#193)
  are planned release-candidate integrations. They are not merged into `main`
  or covered by release CI as of this draft.

## Highlights Since v0.1.6

- Added one platform-neutral launcher and exact-version native packages for
  Windows x64, glibc Linux x64/arm64, and macOS 13+ arm64/x64.
- Added tabbed grids, session resume, the startup grid picker, runtime grid
  resizing, pane worktrees, pane rename/swap/deselect controls, scrollback, and
  wrapped pane navigation.
- Added the dedicated command bar and expanded Alt+C CLI, auth profile
  management and cycling, pane-local manager goals, hidden manager groups,
  idle TODO prompts, and terminal activity/work summaries.
- Added push-to-talk voice input, an offline Linux voice helper, a macOS speech
  helper, and restored Alt+V image paste.
- Improved runtime responsiveness, pane workload protection, usage reporting,
  startup/starter UX, settings presentation, and terminal input reliability.
- Added keyboard-first Pane Activity navigation from merged issue #189: Up/Down
  selects controls, contextual Left/Right changes auth, and Enter/Space acts.
- Added exact-version release stamping, five-platform build/package validation,
  scheduled nightlies, automatic issue labeling, and automated PR review.

## Planned Release-Candidate Integrations

- #187: expand manager goals from pane-local review to stable, validated
  orchestration and targeted follow-ups across relevant live panes.
- #193: give each pane an isolated `CODEX_SQLITE_HOME` while continuing to
  share the user's normal `CODEX_HOME` for auth, config, skills, and sessions.
- Both changes must be reviewed, merged, and revalidated on the release commit
  before `v0.2.0-macos.1` is dispatched.

## Release Channels

- `next`: publish `gridbash@0.2.0-macos.1` and all five native packages from
  one tag and exact version; create a matching GitHub prerelease with tarballs.
- `nightly`: build current `main` daily (or by manual dispatch), publish an
  immutable `0.2.0-nightly...` version under `gridbash@nightly`, and skip an
  unchanged commit. Nightlies do not create release commits or tags on `main`.
- `latest`: leave the stable channel unchanged until the macOS stable-release
  gate is satisfied.

## Validation Plan

- Review and merge #187 and #193, then run `cargo fmt --all -- --check`,
  `cargo clippy --all-targets -- -D warnings`, and `cargo test --no-fail-fast`
  from the integrated release commit.
- Run the launcher, versioning, review-agent, issue-labeler, and star-history
  Node test suites.
- Require the GitHub Actions matrix to build, test, prepare, and package
  Windows x64, Linux x64/arm64, and macOS arm64/x64 from the same commit.
- Verify the launcher and five native manifests/tarballs all contain
  `0.2.0-macos.1`, then verify the npm `next` tag and six GitHub release assets.
- Smoke-test install and startup on each supported platform, including macOS
  microphone/speech permission behavior on real hardware.

## Known Risks

- macOS artifacts are unsigned and not notarized. This release remains a
  preview until Developer ID signing, notarization, and real-hardware smoke
  testing are complete.
- The five native npm package names need a one-time short-lived token bootstrap
  before trusted publishing can manage them. If that npm step fails, the
  workflow still creates or updates the GitHub prerelease, so its tarball
  artifacts persist for inspection and a retry.
