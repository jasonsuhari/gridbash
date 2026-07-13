# GridBash Reference Specification

## Sources

### Existing launch teaser

- File: `docs/assets/gridbash-launch-teaser.mp4`
- Duration / format: 13 seconds, 1920x1080, 30 fps, silent.
- Sampling: 4 fps, 52 measured frames.
- Confidence: measured from the committed render and its HyperFrames source.

### Product demo

- File: `docs/assets/gridbash-openvid-demo.mp4`
- Duration / format: approximately 18 seconds, 1920x1080.
- Sampling: 4 fps, 72 measured frames.
- Confidence: measured from the committed render.

### Talking head

- File: `raw/jason-talking-head.mp4`
- Duration / format: 124.97 seconds, 640x360, 30 fps, AAC 48 kHz stereo.
- Confidence: probed and transcribed with word timestamps.

### Live GitHub repository capture

- URL: `https://github.com/jasonsuhari/gridbash`
- Capture: `capture/github/screenshots/scroll-000.png`, 1920x1080 viewport.
- Captured state: 46 stars, 298 commits, MIT license, v0.1.6 release, active
  Windows/Linux/macOS work visible in repository copy and recent commits.
- Page system: Mona Sans VF 400/500/600; `#FFFFFF`, `#1F2328`, `#59636E`,
  `#0969DA`, `#1F883D`.
- Confidence: live page capture on 2026-07-13; star count is time-sensitive.

## Existing visual system

- Narrative arc: bold category statement -> real product proof -> install CTA.
- Typography: Archivo Black display plus Cascadia Mono operational metadata.
- Palette: canvas `#0b1118`, panel `#121720`, foreground `#f0f4f9`, muted `#8fa2b7`, green `#29d392`, blue `#48a5ff`, amber `#ffca5c`.
- Grid: 58 px safe edge on the existing teaser, top dispatch rail, full-width product plate.
- Camera grammar: static dispatch board with a directional push into product footage and zoom-through to CTA.
- Transition grammar: 0.45-second velocity push, then 0.55-second zoom-through.
- Sound grammar: none in the existing teaser; this is the main system to add.

## Measured cuts

| Cut | Time | Narrative job | Initial -> settled state | Motion | Transfer decision |
| --- | --- | --- | --- | --- | --- |
| 1 | 0.00-3.15 | category hook | blank canvas -> headline and agent roster | stamp/cascade | preserve operational density, replace generic hook with Jason’s line |
| 2 | 3.15-11.05 | product proof | terminal pushes in -> selected broadcast fills frame | directional push and slow product scale | preserve real product dominance and annotations |
| 3 | 11.05-13.00 | CTA | zoom blur -> install plate | zoom-through and lock | extend the hold for a spoken CTA |

## Transfer decisions

- Preserve exactly: palette, font voices, color semantics, real product footage, install command, and the real captured repository page.
- Adapt: expand from three silent beats to six spoken beats; integrate Jason’s real desk as the control station.
- Reject: fake metrics, extra agent logos without narrative purpose, silent delivery, tiny teaser-speed copy.
- Inferred: a rhythmic editorial soundtrack will improve retention without making the product feel like cyberpunk software.
