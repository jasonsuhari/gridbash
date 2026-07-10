# Improve voice transcription accuracy

Date: 2026-07-10
Release target: unreleased
Issue: #130

## Summary

- Replaced legacy Windows speech recognition with modern Windows online dictation.

## What Changed

- Voice mode now uses `Windows.Media.SpeechRecognition` instead of `.NET System.Speech`.
- Increased the end-of-speech pause to make dictated prompts feel less abrupt.
- Added actionable errors for disabled online speech, unsupported languages, and unavailable microphones.
- Updated the voice setup and privacy documentation.

## Why It Matters

- Free-form coding prompts receive substantially better recognition than the legacy desktop engine could provide.

## Validation

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`
- `cargo build --release`
- Confirmed the modern `en-US` dictation grammar compiles on Windows 11.

## Release Notes

- Voice mode now uses modern Windows online dictation for more accurate prompt transcription. Enable Online speech recognition in Windows Settings before using `Alt+Shift+V`.
