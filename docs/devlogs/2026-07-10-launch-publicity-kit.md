# Launch publicity kit

Date: 2026-07-10
Release target: unreleased

## Summary

- Added a launch-ready publicity kit and a short HyperFrames product teaser.
- Updated the README hero to make the core agent-grid workflow understandable
  before a visitor reaches the install instructions.

## What Changed

- Added a 13-second, 1080p launch teaser, poster, reproducible composition
  source, visual design system, and production breakdown.
- Added channel-specific Show HN, Reddit, Product Hunt, social, Discord, and
  technical-article copy with an ordered publication checklist.
- Added response templates for common questions about tmux, platform support,
  orchestration scope, and process lifetime.
- Added a complete technical-article draft about PTYs, input routing, pane-local
  selection, redraw pressure, and git worktree isolation.
- Updated README demo links to lead with the new teaser and point maintainers
  to the launch kit, and updated the GitHub Pages hero to use the same asset.

## Why It Matters

- GridBash now has a concise visual explanation that works without audio and a
  consistent message that can be adapted to each developer community.
- The publication kit emphasizes useful technical context and honest product
  constraints instead of coordinated voting or copy-pasted promotion.

## Validation

- HyperFrames lint: 0 errors, 0 warnings.
- HyperFrames validation: no console errors; 28 text elements pass WCAG AA.
- Rendered the 1920x1080 teaser at 30fps and inspected hook, product-proof, and
  install frames.
- Re-encoded embedded source footage with one-second keyframes to prevent seek
  freezes during deterministic rendering.
- Rendered with one worker, PNG source frames, and software browser compositing;
  inspected frames at 1.5s, 6.5s, 10.0s, and 12.7s.

## Release Notes

- Added launch campaign assets and ready-to-publish community copy for GridBash.
