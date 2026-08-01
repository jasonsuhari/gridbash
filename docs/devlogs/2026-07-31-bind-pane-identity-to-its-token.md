# Bind a pane's control identity to its token

Date: 2026-07-31
Release target: unreleased

## Summary

- Every pane shared one control token and then told the server which pane it
  was. Each pane now holds its own token, and the server reads the caller's
  identity off it.

## What Changed

- `ControlHandle` issues a distinct token per pane and keeps a registry mapping
  each token to the pane it was issued to.
- The control server resolves the presented token to a `ControlCaller`, either
  a pane or the session, and passes that to the app. The `caller_pane_id` field
  is gone from the wire format, and a request that still carries one is
  authorised as whoever its token says.
- A pane's token is dropped when its pane goes away, alongside the workload and
  render-cache entries that are already retired there.

## Why It Matters

- `prompt --others` resolves its target list from the caller's identity. That
  identity arrived in the request body, filled in from the caller's own
  `GRIDBASH_PANE_ID`, and nothing checked it. A pane could name a neighbour and
  redirect a broadcast: excluding a pane it is not, and including itself.
- Sharing one credential across every pane also meant nothing could be
  attributed or revoked. Each pane now carries a credential that identifies it
  and expires with it.
- This does not stop a pane from prompting its neighbours, which is the feature.
  It stops a pane from lying about which one it is.

## Validation

- `cargo test --bin gridbash -- --test-threads=1`: 401 passed, 5 ignored.
- `cargo clippy --bin gridbash --all-targets -- -D warnings` and
  `cargo fmt --check` are clean.
- New tests cover token-to-pane resolution, a request that declares a different
  caller than its token proves, and revocation when a pane is retired. The
  resolution test was confirmed to fail when the mapping is perturbed.

## Release Notes

- A pane can no longer present itself to the agent control API as a different
  pane.
