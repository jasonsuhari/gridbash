# Fix low usage percentage scaling

Date: 2026-07-12
Release target: unreleased

## Summary

- Corrected auth-profile quota displays for usage values at or below 1%.

## What Changed

- Treat Anthropic usage `utilization` values consistently as percentages on the documented 0–100 scale.
- Added regression coverage for 0.5%, 1%, and 73% utilization values.

## Why It Matters

- Freshly reset accounts no longer appear nearly exhausted when they have consumed less than 1% of a usage window.

## Validation

- `cargo test auth::tests::treats_low_usage_utilization_as_a_percentage`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt -- --check`
- `git diff --check`

## Release Notes

- Fixed incorrect quota percentages for auth profiles with very low utilization.
