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

## Why It Matters

- Linux developers can run the same PTY-backed GridBash workflow locally, through tmux, or over SSH.

## Validation

- `npm run test:launcher` (8 passing tests)
- `npm pack --dry-run --ignore-scripts`

## Release Notes

- GridBash now supports Windows x64 plus glibc Linux on x64 and arm64.
