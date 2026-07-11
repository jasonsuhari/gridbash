# GridBash: Six Agents, One Terminal

## Style block

Create a silent 13-second, 1920x1080 launch teaser set inside an operator
dispatch board for coding agents. Use the exact GridBash palette: canvas
`#0b1118`, panel `#121720`, foreground `#f0f4f9`, muted copy `#8fa2b7`,
GridBash green `#29d392`, focus blue `#48a5ff`, and completion amber `#ffca5c`.
Use Archivo Black for statements and Cascadia Mono for operational copy.

## Rhythm

`SLAM -> proof -> PEAK -> hold`

The hook occupies 0.0-3.4 seconds, live product proof occupies 3.4-11.1
seconds, and the install CTA occupies 11.1-13.0 seconds.

## Global rules

- Three depth layers in every beat: atmospheric routing grid, primary content,
  and foreground status rails/registration marks.
- Keep 8-10 visible elements per beat without allowing decoration to compete
  with the message.
- Primary transition is a 0.45-second directional push with blur,
  `power3.inOut`. The CTA uses a 0.55-second zoom-through, `expo.inOut`.
- All entrances are deterministic `fromTo` tweens on the paused root timeline.
- No narration or music. The asset must work muted on GitHub, Hacker News,
  Product Hunt, X, LinkedIn, and Reddit.
- The real committed GridBash demo video is the only product footage.

## Beat 1 - The dispatch order (0.0-3.4s)

### Concept

The frame feels like an operations board waking up for a shift. Six agent names
route into place around a massive command: "SIX AGENTS. ONE TERMINAL." The
viewer should understand the category and benefit before seeing the product.

### Mood direction

Airport departures board meets a disciplined terminal operations room. Precise,
compact, and credible rather than futuristic.

### Depth layers

- BG: offset routing grid DRIFTS horizontally; green radial wash BREATHES;
  oversized grid coordinates HOLD at low opacity.
- MG: headline STAMPS from the left; subhead LOCKS under it; six agent labels
  ROUTE into two uneven rows.
- FG: top status rail DRAWS across; issue-like sequence number TICKS on;
  registration corners SNAP into place.

### Choreography

- `GRIDBASH / AGENT DISPATCH` types on in Cascadia Mono.
- `SIX AGENTS.` stamps in from the left with `expo.out`.
- `ONE TERMINAL.` rises with a small scale correction and `back.out(1.15)`.
- Agent labels cascade from alternating directions in under 450ms.
- The routing rule draws left-to-right while the background grid drifts once.

### Transition out

Directional push at 3.15s. Beat 1 accelerates left with 8px blur while the live
terminal window pushes in from the right and resolves sharply over 0.45s.

## Beat 2 - Product proof (3.15-11.1s)

### Concept

The actual GridBash terminal grid becomes the control-room window. Operational
labels point at the three differentiators visible in the footage: multiple live
PTY panes, selected broadcast, and isolated git worktrees.

### Mood direction

Raw developer demo with editorial annotation. The product is not placed inside
a fake laptop or browser; it is the instrument panel.

### Depth layers

- BG: canvas and routing grid remain visible at the margins; blue focus wash
  PULSES once behind the terminal.
- MG: terminal window PUSHES into its hero position and slowly advances through
  real committed footage.
- FG: `REAL PTYS`, `SELECTED BROADCAST`, and `ISOLATED WORKTREES` labels LOCK to
  frame edges; a `6 LIVE PANES` counter STAMPS in amber; a green scan rule
  TRAVELS once across the product window.

### Choreography

- Terminal wrapper enters from x=520 with scale 0.9 and `power4.out`.
- Product video remains unmodified other than perspective-free framing and a
  slow 1.00-to-1.025 child scale.
- Annotation labels enter from three different directions using expo, circ,
  and power eases.
- Scan rule traverses once; no infinite loops or decorative particles.

### Transition out

Zoom-through at 11.15s. The terminal expands toward the viewer and blurs to
18px while the CTA resolves from 0.78 scale to full size over 0.55s.

## Beat 3 - Install and run (11.05-13.0s)

### Concept

The visual noise falls away. The install command becomes a physical command
plate, with the repository URL and platform status supporting it. This is the
only centered beat, used deliberately for closure and copyability.

### Mood direction

Confident release card, closer to a stamped shipping label than a SaaS CTA.

### Depth layers

- BG: routing grid holds; amber completion wash breathes once; giant cropped
  `READY` ghost type drifts by a few pixels.
- MG: `RUN THE GRID.` stamps in, command plate draws open, install command types
  on as a single readable unit.
- FG: `OPEN SOURCE / MIT`, Windows x64 strip, GitHub URL, registration corners, and
  a green completion rule snap into place.

### Choreography

- Headline drops from above with `expo.out`.
- Command plate scales from 0.92 while its border draws with `circ.out`.
- Install command enters from x=-40 with no typewriter gimmick so it remains
  readable in still frames.
- URL and metadata rise on different timings; green rule resolves last.
- Final 0.4 seconds hold at full clarity. No mandatory fade to black.

### Transition out

None. Hold the complete CTA through the final frame.

## Recurring motifs

- A top-left dispatch identifier changes from `DISPATCH 01` to `LIVE GRID 06`
  to `READY 01`.
- Thin green routes connect claims to the terminal frame.
- Blue appears only for active focus; amber appears only for completion.
- Cropped monospace coordinates keep the dispatch-board metaphor consistent.

## Negative prompt

No neon purple/blue gradient, gradient text, glassmorphism, fake charts, fake
metrics, glowing orbs, HUD circles, particle fields, tiny web typography,
equal-weight card grids, generic centered hero layout, or invented product UI.
