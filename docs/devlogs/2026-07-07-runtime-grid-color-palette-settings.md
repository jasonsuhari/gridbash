# Runtime grid color palette settings

Date: 2026-07-07
Release target: unreleased

## Summary

- Added live settings for GridBash grid color roles.
- References issue #34.

## What Changed

- Replaced the sample settings rows with runtime palette controls.
- Added palette roles for accent/chrome, focus border, selected border, active border, quiet border, and exited border.
- Settings rows show a color swatch and current color name.
- Palette changes apply immediately while GridBash is running.
- Quiet panes now default to a sky-blue border instead of violet.

## Why It Matters

- Users can tune loud or ugly colors without changing code.
- Quiet-output indicators stay noticeable while still fitting the user's preferred GridBash palette.

## Validation

- Ran `cargo fmt --check`.
- Ran `cargo clippy -- -D warnings`.
- Ran `cargo test`.

## Release Notes

- Added runtime GridBash palette settings for the existing grid color roles.
