# GridBash Feature Launch — Operations Field Manual

## Visual world

The film returns to the original launch video's tactile cutting table, now
covered by a complete GridBash operations manual. Thick paper sections unfold,
terminal printouts lock into registration marks, tab dividers expose whole
feature families, and mint route lines connect actions to the panes they affect.
The sequel should feel like a skilled motion designer expanded the first film's
visual system, not like a separate campaign.

The camera is orthographic and overhead. Product proof stays square-on and
sharp. The only sense of depth comes from layered paper, physical crop marks,
offset shadows, and foreground tabs crossing the frame. There are no synthetic
camera flights and no fake terminal screenshots.

## Palette

- Paper: `#F3EDE1`
- Ink: `#171817`
- Soft ink: `#2B2C2A`
- GridBash mint: `#78C9B6`
- Tab yellow: `#E2C94C`
- Paper highlight: `#FFF9EE`

Use only these colors in composition CSS. Alpha values may be derived from
them. Real GridBash footage keeps its source colors.

## Typography

- Display voice: `Anton`, 400, tracking `-0.025em`, line height `0.88`.
- Operational voice: `Geist Mono`, 400/700, tracking `0.01em` to `0.06em`.
- Display statements: 112–196 px.
- Feature labels: 26–34 px.
- Supporting copy: 30–42 px.
- Terminal and metadata labels: 20–27 px.

Anton is the physical poster voice; Geist Mono is the operating manual.
Headlines stay short, asymmetrical, and edge-anchored. Time provides hierarchy:
one claim lands first, the proof plate follows, and supporting labels register
last. Avoid centered web-style stacks.

## Frame and material

- Master: 1920×1080, 30 fps, 96 seconds.
- Paper canvas carries subtle deterministic fiber/noise, never a gradient.
- Paper shapes use 18–34 px corners and a soft down-right offset shadow.
- Every scene has background paper, a middle proof or schematic layer, and
  foreground tabs/rules/registration marks.
- Real product footage is the only dense surface and remains unwarped.
- UI schematics must quote current product labels and behavior from
  `README.md`, `docs/REFERENCE.md`, and `src/ui.rs`; they are illustrative
  callouts around real footage, not fake screen captures.

## Motion language

- Primary transition: two full-frame paper panels cover the outgoing scene,
  switch the scene while covered, then peel away in 0.46 seconds with
  `power4.inOut`.
- Accent transition: vertical manual-page push in 0.50 seconds with
  `power3.inOut` at section changes.
- Final transition: ink-color dip over 0.72 seconds.
- Motion verbs: cut, route, stamp, fold, register, peel, stack, snap, settle.
- First motion begins at 0.18 seconds.
- Entrances use three or more distinct directions/eases per scene, then settle
  completely for reading.
- Ambient motion is limited to one slow route-line drift, paper-tab breath, or
  proof-plate push per scene and is always attached to the main timeline.

## Sound language

- Text-led feature film with no voiceover required.
- A restrained original percussion-and-pulse bed supports the 96-second arc.
- Paper hits mark scene reveals; route ticks mark targeted dispatch; a low
  mechanical fold marks tabs, resize, and recovery.
- Sound effects stay below the information layer and never become cartoonish.

## Do

- Reuse the approved real six-pane product capture for the core workflow.
- Show the complete current product in grouped, readable feature chapters.
- Keep claims synchronized with v0.2.0 and the current reference docs.
- Hold commands and feature labels long enough to read without audio.
- Use exact current shortcuts and tool names only when they materially clarify
  the feature.
- End on `npm install -g gridbash` and the GitHub repository.

## Do not

- No neon, HUD rings, gradients, glass, glossy 3D, particles, or gradient text.
- No generic SaaS card grid, identical dashboard tiles, or tiny web UI type.
- No generated terminal text, fake GitHub metrics, or unsupported claims.
- No centered equal-weight feature wall.
- No repeated entrance recipe within a scene.
- No jump cuts, infinite repeats, wall-clock animation, or non-seekable media.
- No `<br>` in body copy; display phrases may use separate block spans only.
