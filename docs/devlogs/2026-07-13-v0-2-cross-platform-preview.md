# v0.2 cross-platform preview

Date: 2026-07-13
Issue: #192
Release target: v0.2.0-macos.1

## Summary

- Aggregate the product, platform, and release work on `main` since `v0.1.6`
  into one cross-platform preview.
- Keep the release a prerelease while the macOS signing gate is open.
- Include the completed manager-grid orchestration (#187), Pane Activity
  navigation (#189 via PR #201), and per-pane Codex SQLite isolation (#193).

## Highlights Since v0.1.6

- Added one platform-neutral launcher and exact-version native packages for
  Windows x64, glibc Linux x64/arm64, and macOS 13+ arm64/x64.
- Added tabbed grids, bounded session resume, runtime grid resizing, pane
  worktrees, pane rename, scrollback, and wrapped pane navigation; refined the
  existing startup grid picker.
- Added the dedicated command bar and expanded Alt+C CLI, auth profile
  management and cycling, a configurable live-grid palette, hidden manager
  groups, idle TODO prompts, and terminal activity/work summaries.
- Expanded manager goals into grid-wide orchestration with strict response
  validation, stable per-pane dispatch tracking, targeted follow-ups, and
  bounded recovery after partial or asynchronous write failures.
- Isolated Codex SQLite goals, memories, and thread relationships per pane with
  a persistent `CODEX_SQLITE_HOME`, including shell-profile launches and pane
  restarts. The normal `CODEX_HOME` remains shared for auth, configuration,
  skills, and rollout history, and explicit user overrides remain authoritative.
- Added push-to-talk voice input, an offline Linux voice helper, a macOS speech
  helper, and restored Alt+V image paste. Pane shells now inherit the invoking
  shell consistently across supported platforms.
- Improved runtime responsiveness, pane workload protection, usage reporting,
  startup/starter UX, settings presentation, and terminal input reliability.
- Added keyboard-first Pane Activity navigation (#189 via PR #201): Up/Down
  selects controls, contextual Left/Right changes values, and Enter/Space acts.
- Added exact-version release stamping, five-platform build/package validation,
  scheduled nightlies, automatic issue labeling, and automated PR review.

## Integrated Release-Candidate Work

- #187: manager goals now coordinate relevant live panes, validate manager
  responses, and retry only work that was not confirmed written.
- #193: each pane receives an isolated, leased SQLite lane while sharing the
  user's normal Codex home and respecting explicit SQLite-home overrides.
- Both changes were independently reviewed, merged through the five-platform
  pull-request matrix, and revalidated together on `main`.

## Release Channels

- `next`: publish the root launcher plus all five native packages at the exact
  `0.2.0-macos.1` version under npm's `next` tag. Create GitHub prerelease
  `v0.2.0-macos.1` with the six matching package tarballs.
- `nightly`: build current `main` daily at 08:17 UTC or by manual dispatch,
  publish an immutable `0.2.0-nightly.YYYYMMDD.RUN.g<sha>` version under npm's
  `nightly` tag, and create a GitHub prerelease tag pointing at that commit.
  Version stamping is workspace-only, so nightlies do not add a version commit
  to `main`. An unchanged commit is skipped only when both npm and the GitHub
  release already represent it.
- `latest`: leave the stable channel unchanged until the macOS stable-release
  gate is satisfied.

## Validation Plan

- Run `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
  and `cargo test --no-fail-fast` from the integrated release commit.
- Run the launcher, versioning, review-agent, issue-labeler, and star-history
  Node test suites.
- Require pull-request CI to build and test Windows x64, Linux x64/arm64, and
  macOS arm64/x64. Separately require the Release workflow to prepare and pack
  all five native binaries plus the platform-neutral launcher from one commit.
- Verify `Cargo.toml`, the root `package.json`, all five native manifests, and
  all optional native dependency pins contain the exact release version.
- Verify exactly six `.tgz` assets on `v0.2.0-macos.1`, then verify all six npm
  package versions and `next` dist-tags.
- Manually dispatch a forced nightly from integrated `main`, verify its GitHub
  prerelease and assets, then rerun unchanged to exercise the skip condition.
- Smoke-test install and startup on each supported platform, including macOS
  microphone/speech permission behavior on real hardware.

## Known Risks

- macOS artifacts are unsigned and not notarized. This release remains a
  preview until Developer ID signing, notarization, and real-hardware smoke
  testing are complete.
- The five native npm package names need a one-time short-lived token bootstrap
  before trusted publishing can manage them. After the first publication,
  configure the GitHub Actions trusted publisher for every package and remove
  the temporary secret. If npm publication fails, the workflow still creates
  or updates the GitHub prerelease so its tarballs persist for inspection and
  a retry.
