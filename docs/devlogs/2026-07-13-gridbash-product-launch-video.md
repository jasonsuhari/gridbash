# GridBash product launch video

Date: 2026-07-13
Release target: unreleased

## Summary

- Added a 45-second, 1920x1080 GridBash product-launch film built from Jason's
  talking-head recording and real GridBash product footage.
- Added the production brief, measured music/reference maps, taste rules, scene
  plan, approval record, and critic template used to make the film repeatable.

## What Changed

- Selected the strongest non-repeated takes from the source recording and
  assembled a clean, silence-free voice timeline without an intermediate lossy
  encode.
- Built a deterministic HyperFrames composition with six narrative scenes,
  word-timed captions, semantic sound effects, beat-aware transitions, and
  GridBash's terminal-native visual system.
- Recorded six genuine Codex sessions in six disposable git worktrees, then
  demonstrated subset routing by sending one real prompt only to panes 2, 3,
  5, and 6 and capturing their independent replies.
- Added a reproducible talking-head preparation script and edit manifest so the
  source cut and caption timing can be regenerated.
- Documented third-party music and sound-effect sources and their licenses.

## Why It Matters

- GridBash now has a launch asset that explains its core value—routing multiple
  coding-agent sessions into isolated worktrees—instead of showing an unexplained UI.
- The film uses product proof and the founder's actual voice rather than fake
  metrics, synthetic UI, or unsupported claims.

## Validation

- HyperFrames lint: 0 errors; one accepted maintainability warning for the
  standalone composition size.
- Runtime validation: no browser console errors; all 52 measured text elements
  pass WCAG AA contrast.
- Layout inspection: seven targeted product-proof samples plus distributed scene
  checks, with no overflow or unintended occlusion findings.
- Animation map: 591 tweens inspected; intentional transition collisions and
  readable holds documented, with caption-zone overlap corrected.
- Final media: 1920x1080 H.264/yuv420p at 30 fps, AAC stereo at 48 kHz,
  45.184 seconds, 23,158,646 bytes.
- Final mix: -21.9 dB mean and -2.1 dB peak, with no clipping.
- Final encoded-frame review confirmed that the live GridBash capture advances
  correctly through selection, prompt submission, and independent responses.

## Release Notes

- The spoken cross-platform claim reflects the current packaging work on
  `main`; publish the film only alongside Windows, Linux, and macOS artifacts.
