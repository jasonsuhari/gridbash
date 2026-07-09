# Add voice mode

Date: 2026-07-09
Release target: unreleased

## Summary

- Added cancellable push-to-talk dictation for GridBash terminal and command-bar input.

## What Changed

- Added `Alt+v` to listen for one utterance with the installed Windows speech recognizer.
- Preserved the command bar or pane targets selected when listening starts and never auto-submitted dictated text.
- Added cancellation, no-speech, unavailable-recognizer, microphone-error, and changed-tab status handling.
- Added a visible `MIC` state plus shortcut and setup documentation.

## Why It Matters

- Longer agent prompts can be dictated without leaving the keyboard-driven GridBash workflow or risking immediate command execution.

## Validation

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test` (98 passed, 1 existing interactive ConPTY smoke test ignored)
- `cargo build --release`
- Confirmed `System.Speech` loads and one Windows speech recognizer is installed on the development machine.

## Release Notes

- Press `Alt+v` to dictate into the current command bar or pane targets. Press it again to cancel; review the inserted text and submit it normally when ready.
