# GridBash Product Launch — Tactile Cut-Paper System

## Visual world

The film looks hand-built in After Effects from thick paper and matte clay sheets.
Shapes are flat and graphic, but soft offset shadows give them one shallow layer
of depth. GridBash terminal footage is the only dense surface; everything around
it is quiet, oversized, and deliberately composed.

The dominant metaphor is an editor's cutting table: cards slide into register,
routes draw like cord laid on paper, windows stack, and branch strips peel away.
Motion is orthographic and shape-led. There are no synthetic camera flights.

## Palette

- Paper: `#F3EDE1`
- Ink: `#171817`
- Soft ink: `#2B2C2A`
- GridBash mint: `#78C9B6`
- Tab yellow: `#E2C94C`
- Paper highlight: `#FFF9EE`

Use only these colors in composition CSS. Alpha values may be derived from them.
Captured camera, product, and GitHub plates keep their real source colors.

## Typography

- Display voice: `Anton`, 400, tracking `-0.025em`, line height `0.88`.
- Operational voice: `Cascadia Mono`, 400/700, tracking `0.01em` to `0.06em`.
- Display statements: 118–210 px.
- Captions: `Anton`, 60–78 px, paper fill with a heavy ink outline; no pill.
- Operational labels: 23–31 px.

Headlines are short, cropped, and asymmetrical. Time is hierarchy: one phrase
lands, holds, then makes room for the next. Never build centered web-style stacks.

## Frame and material

- Master: 1920×1080, 30 fps.
- Paper canvas carries subtle deterministic fiber/noise, never a gradient.
- Clay/paper shapes use 18–34 px corners and a soft down-right offset shadow.
- Real UI remains square-on, sharp, and unwarped.
- Jason's footage remains the original full frame or a simple rounded crop. Never
  redraw his face, remove his background, or place him on a synthetic scene.
- Every hero frame has background paper, a content plate, and foreground tabs or
  route strokes, but no decorative filler.

## Motion language

- Primary transition: a paper panel or oversized glyph wipes the full frame in
  0.28–0.46 seconds with `power4.inOut`.
- Accent transition: a layered paper peel reveals the product in 0.55–0.72 seconds.
- Motion verbs: cut, stack, register, peel, route, snap, stamp, settle.
- Entrances use visible overshoot followed by complete stillness.
- Repeated cards enter on different axes and with different timing.
- Route strokes draw only when the narration describes routing or branching.
- First motion begins at 0.18 seconds.

## Sound language

- Keep the existing voice, music, and restrained SFX map.
- A pop means a card lands. A route tick means a connection is made.
- The branch whoosh is reserved for the worktree transformation.
- SFX sit beneath the voice and never imitate cartoon boings.

## Do

- Use the two approved Seedance plates only as tactile motion plates.
- Composite exact HTML text and exact real UI over any generated plate as needed.
- Let Jason's red hoodie remain the human color contrast.
- Use the real product demo for selection, focus, routing, and independent replies.
- Recapture GitHub immediately before final render and use the live star count.
- Hold `npm install -g gridbash` long enough to copy.

## Do not

- No chrome, glass, glossy 3D, fake cinematic cameras, neon, HUDs, or particles.
- No generic SaaS card grids, pill captions, gradient text, or tiny metadata.
- No generated readable terminal UI, GitHub UI, commands, or factual claims.
- No residual wobble after an element settles.
- No cross-platform availability claim in this cut. The published `gridbash@0.1.6`
  package is Windows-only and the macOS/Linux platform packages are not live.
- Use `assets/voice-truth-safe.wav`, which removes the inaccurate OS clause and
  joins “completely open source” to “You can install it…” with a short natural pause.
