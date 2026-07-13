# Automatic PR review agent

Date: 2026-07-13
Release target: unreleased

## Summary

- Add a repository-owned AI reviewer that scans every ready pull request and
  maintains one concise review report.

## What Changed

- Added a trusted-base GitHub Models workflow with minimal permissions and
  manual recovery dispatch.
- Added a dependency-free, tested reviewer that paginates changed files,
  bounds model input, resists prompt injection, and upserts its report.
- Added GridBash-specific review instructions and setup/security documentation.
- Added explicit truncated/omitted filename reporting and a configurable Models
  API version after dogfooding the live reviewer on its own implementation PR.

## Why It Matters

- Pull requests now receive a semantic correctness and security pass in
  addition to compilation, tests, formatting, DCO, and secret scanning.
- Contributors can get feedback without requiring a CodeRabbit installation or
  storing a third-party model key.

## Validation

- `npm run test:review-agent`
- `npm run test:launcher`
- `cargo fmt -- --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- Live workflow dispatch against the implementation pull request after merge.

## Release Notes

- Pull requests now receive an automatic GridBash-specific AI review report.
