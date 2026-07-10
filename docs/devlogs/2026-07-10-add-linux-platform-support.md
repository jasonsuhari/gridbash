# Add Linux platform support

Date: 2026-07-10
Release target: unreleased
Issue: #141

## Summary

- Add native Linux x64 and arm64 support without forking the Windows codebase or release line.

## What Changed

- Added a shared target manifest for Windows x64 and glibc Linux on x64 and arm64.
- Updated the npm launcher to select the packaged executable from the current platform and architecture.
- Generalized native package preparation and local installation for supported targets.
- Expanded npm metadata and binary ignore rules for the Linux artifacts.
- Added glibc Linux packages for x64 and arm64 with both the GridBash TUI and an
  offline Whisper voice helper.
- Added bash, zsh, fish, sh, and PowerShell profiles on Unix PTYs, plus SSH/tmux
  compatibility guidance and a real `--no-mouse` fallback.
- Added explicit first-use consent, cancellation, size checking, and SHA-256
  verification for the offline voice model.

## Why It Matters

- Linux developers can run the same PTY-backed GridBash workflow locally, through tmux, or over SSH.

## Validation

- Five-platform CI matrix covering Windows x64, Linux x64/arm64, and macOS x64/arm64
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `npm run test:launcher`
- Native npm package dry-runs on each target

## Release Notes

- GridBash now supports Windows x64 plus glibc Linux on x64 and arm64.
