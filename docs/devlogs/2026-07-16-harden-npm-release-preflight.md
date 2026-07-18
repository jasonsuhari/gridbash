# Harden npm release preflight

Date: 2026-07-16
Release target: unreleased

## Summary

- Added an npm registry preflight ahead of all cross-platform release builds.
- Documented the ownership and trusted-publisher setup needed after npm creates
  or transfers a package.

## What Changed

- The release workflow now verifies all six npm package names, the
  `jasonmatthewsuhari` owner, and the GridBash GitHub repository before starting
  the native build matrix.
- The preflight recognizes npm's `0.0.1-security` placeholder only after the
  expected owner controls it, allowing the first legitimate package publish to
  replace the placeholder metadata.
- Focused Node tests cover package discovery, owner parsing, repository
  normalization, transferred placeholders, and drift failures.

## Why It Matters

- Registry ownership problems now fail in seconds instead of after five native
  builds. The documented trust audit also closes the authentication step that
  remains after npm Support transfers a blocked name.

## Validation

- `npm run test:release-preflight`
- `node npm/scripts/release-preflight.js`
- `npm run test:version`

## Release Notes

- Release automation now checks npm ownership before building publish artifacts.
