# Rebuild launch video motion direction

Date: 2026-07-13
Release target: unreleased

## Summary

- Rebuilt the 45-second GridBash launch film around a tactile cut-paper/clay motion
  system, real talking-head footage, and the real six-pane product capture.
- Replaced the rejected dark terminal-HUD treatment with warm paper, ink, mint,
  yellow proof tabs, Anton display type, and deterministic GSAP choreography.

## What Changed

- Added two 1920x1080 render-safe Seedance motion plates for the hook and product
  reveal. The two Higgsfield generations used 25 credits total; no variants or
  retries were generated.
- Added local Anton and Montserrat font assets and rewrote the HyperFrames
  composition, production brief, scene plan, script, and design specification.
- Kept generated footage illustrative only. Product routing, worktrees, GitHub,
  the install command, and every factual claim use real captures or HTML.
- Refreshed the GitHub receipt immediately before rendering. The current count remains
  visible in that receipt; the supporting HTML stamp uses the evergreen `LIVE REPO` label.
- Re-encoded the exact product capture with one-second keyframes so HyperFrames can
  seek it deterministically without freezing.
- Cut the recorded “Mac, Linux, and Windows” clause from the public audio. The
  published `gridbash@0.1.6` package is still Windows x64 and the macOS/Linux npm
  packages were not live at render time.

## Why It Matters

- The launch film now has one coherent, manually directed visual language instead
  of generic AI chrome and HUD decoration.
- Viewers see the real product doing the real routing workflow, while the generated
  motion is limited to transitions and visual setup.
- The public cut makes only claims supported by the current GitHub and npm state.

## Validation

- `npm run lint` — 0 errors, 0 warnings.
- `node node_modules/hyperframes/dist/cli.js validate --timeout 30000` — passed.
- `npm run inspect` — 0 layout issues across 15 timeline samples.
- Draft render reviewed at nine representative timestamps plus a six-frame product
  reveal contact sheet.
- Standard render completed at 1920x1080, 30 fps, H.264/AAC, 45.184 seconds.
- Final audio measured -22.1 dB mean and -2.1 dB max; the truth-safe edit contains
  a 224 ms natural pause between “open source” and “You can install it.”
- Rendered hook, problem, product-reveal, receipt, and CTA filmstrips from the encoded
  MP4. Corrected a delayed-tween initial-state leak and covered generated pseudo-UI
  before it became legible.
- HyperFrames parallel capture was unstable on Windows during the final text-only
  revision. The reviewed master was patched at the receipt stamp with FFmpeg at CRF 18;
  its validated AAC stream was copied unchanged.

## Release Notes

- Replaces `docs/assets/gridbash-product-launch-video.mp4`.
- No application runtime behavior changed.
