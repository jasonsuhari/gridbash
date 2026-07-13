# GridBash: The Token-Maxing Operator Station

## Style block

Create a 45.16-second, 1920x1080, 30 fps talking-head product launch film. The
visual world is Jason’s real desk becoming an operator dispatch station for CLI
coding agents. Preserve the exact palette from `design.md`: canvas `#0b1118`,
panel `#121720`, foreground `#f0f4f9`, muted `#8fa2b7`, GridBash green
`#29d392`, focus blue `#48a5ff`, and completion amber `#ffca5c`. Use Archivo
Black for display statements and Cascadia Mono for operational copy.

## Rhythm

`HOOK -> human problem -> PRODUCT PEAK -> proof build -> isolation transform -> CTA hold`

Scene boundaries land at measured soundtrack onsets: 6.32, 13.75, 20.04,
29.75, 37.18, and 45.16 seconds.

## Global rules

- Use Jason’s original camera frame as the human foundation. No face matting or synthetic background replacement.
- Use three depth layers: real camera/product atmosphere, primary content, and foreground routing/caption accents.
- Treat the 640x360 source as an intentional broadcast plate when product detail shares the frame; inspect the 1080p upscale at 100%.
- Use conversational 2-4 word caption groups synced to remapped word timestamps. Highlight `CLAUDE`, `GRIDBASH`, `BROADCAST`, `WORKTREE`, and the OS names with semantic brand colors.
- Keep one caption group visible at a time and hard-kill it at the group end.
- Use the licensed soundtrack “Close Up” by Michael Ramir C.; dialogue remains dominant.
- Use one semantic SFX family per meaning. Vary repeated selection clicks slightly in pitch.
- Use deterministic `fromTo` tweens on one paused root GSAP timeline. No runtime randomness or infinite loops.
- Every scene receives entrance choreography. Scene transitions handle exits; only the final scene may fade.
- Do not invent product UI, metrics, customer logos, or autonomous agent coordination.

## Scene 1 — Ten Claude sessions (0.00-6.32)

### Concept

Jason is already at his real desk, mid-confession. The room feels physical and
warm, but a terse GridBash dispatch rail draws across the frame as he says the
opening joke. A giant `10` stamps into the dark right side of the room, followed
by `CLAUDE SESSIONS` as operational labels. The joke should feel knowingly
unhinged, not like a fake performance metric.

### Depth layers

- BG: real studio frame, restrained canvas tint at the edges, fine deterministic grain, one green route wash tied to the laptop.
- MG: Jason and laptop remain the human focal point; `10` occupies the dark right wall without covering his face.
- FG: top dispatch rail, `TOKENMAX MODE` label, 2-4 word captions, small session ticks that count to ten.

### Choreography

- Dispatch rail DRAWS at 0.18s with `power3.out`.
- `10` STAMPS on the 1.74s onset with 8-frame overshoot and a two-frame positional recoil.
- Session ticks ROUTE in under 420ms using alternating horizontal directions.
- Captions LOCK by phrase; `CLAUDE` uses green, `10` uses amber.
- One subtle camera-plate push runs across the scene; it fully settles before transition.

### Transition

Velocity push at 6.32s: terminal plates enter from the right while the camera
content shifts left under 10px blur, 0.40s, `power3.inOut`.

## Scene 2 — Why Jason built it (6.32-13.75)

### Concept

The operator station becomes overloaded. Terminal plates multiply from the
laptop into the room’s open zones as Jason describes token-maxing and losing
track of each terminal. They begin misaligned and difficult to scan, then the
GridBash routing system snaps them into one disciplined 2x3 silhouette.

### Depth layers

- BG: camera plate shifts to the right 58% of frame; oversized low-opacity path labels drift once.
- MG: five terminal crops CASCADE around the laptop without covering Jason’s face.
- FG: `TOO MANY WINDOWS` stamp, connection rules, active terminal counter, synced captions.

### Choreography

- Each terminal plate CASCADES from a different edge with expo/circ/back entrance eases.
- Plates remain still after their entrance; do not add wobbly residual motion.
- At “forgetting what I was doing,” labels briefly desynchronize, then SNAP into a 2x3 outline on 13.75s.
- Use routed tick sounds, not repeated whooshes.

### Transition

Zoom-through at 13.75s. The empty 2x3 outline expands into the real GridBash
product frame over 0.48s, `expo.inOut`, while Jason resolves into a side plate.

## Scene 3 — GridBash reveal (13.75-20.04)

### Concept

This is the product peak. `GRIDBASH` locks to the top rail as the real product
footage becomes the instrument panel. Jason remains visible in a compact
broadcast plate long enough to preserve the human introduction, then the
product earns the frame.

### Depth layers

- BG: canvas routing grid and a single green radial wash.
- MG: real product demo, square-on and legible; Jason’s camera plate on the right for the first half.
- FG: `GRIDBASH / AGENT GRID`, `CLI CODING AGENTS`, route coordinates, captions.

### Choreography

- Wordmark LOCKS from 0.96 scale with the largest impact at 13.75s.
- Product frame PUSHES from the laptop’s screen position, not from an arbitrary edge.
- Jason’s plate holds; no decorative cutout or face matting.
- At “beautiful grid,” six pane outlines DRAW in sequence and stop.

### Transition

Directional push into full product proof at 20.04s, 0.36s, `power3.inOut`.

## Scene 4 — Route exactly where it belongs (20.04-29.75)

### Concept

The film stops describing and proves the interaction. Full-frame product
footage shows the terminal sessions, focus border, selected panes, and broadcast
bar. Editorial labels attach only to visible behavior.

### Depth layers

- BG: real product footage fills the safe frame.
- MG: selective 1.00-to-1.025 child scale follows the broadcast action, then settles.
- FG: `REAL TERMINAL SESSIONS`, `FOCUS ONE`, `SELECT A FEW`, `BROADCAST` callouts and captions.

### Choreography

- `REAL TERMINAL SESSIONS` LOCKS at 20.04s.
- Focus blue SNAPS to one pane with a click.
- Green selection states CASCADE across the exact selected panes with pitch-varied clicks.
- Broadcast label and bar LAND together on the spoken word `broadcast`.
- No invented cursor movement or fake command output.

### Transition

The selected pane borders extend outward into four parallel branch lanes at
29.75s. Use a velocity-matched vertical push, 0.42s, `power3.inOut`.

## Scene 5 — Worktree isolation (29.75-37.18)

### Concept

The selected grid becomes a Git branch map. Each chosen pane receives a
repo-local worktree label and its own route. The metaphor stays operational:
parallel work is separated cleanly, not represented by generic floating cards.

### Depth layers

- BG: dimmed product grid and restrained branch coordinates.
- MG: four selected pane crops remain anchored to their real positions.
- FG: branch lines, `WORKTREE 01-04` labels, repo paths, captions, one `NO FILE FIGHTS` resolution stamp.

### Choreography

- Branch routes DRAW from the selected pane borders on the 29.75s onset.
- Worktree labels DROP in with stagger under 450ms.
- A single branch whoosh lands when the four routes become independent.
- `same exact files` resolves as the routes stop moving and the amber completion ticks appear.

### Transition

The branch lanes compress into the strokes of the install command at 37.18s,
0.44s, `power3.inOut`.

## Scene 6 — Open source, run the grid (37.18-45.16)

### Concept

Jason returns in a narrow real-camera plate as the command becomes the closing
object. The frame reads like a stamped open-source release card: confident,
copyable, and honest. Mac, Linux, and Windows appear only because publication is
gated on real packages for all three.

### Depth layers

- BG: canvas grid, giant cropped `READY` ghost type, one amber completion wash.
- MG: Jason’s camera plate and the install command plate.
- FG: `OPEN SOURCE / MIT`, OS labels, repository URL, green completion rule, final captions.

### Choreography

- `OPEN SOURCE` STAMPS at 37.18s.
- Command plate EXPANDS from the compressed branch lanes; border DRAWS once.
- Display `npm install -g gridbash` as one readable unit while Jason says the shorter spoken version.
- `MAC`, `LINUX`, and `WINDOWS` LOCK on their spoken words.
- Repository URL rises last; green completion rule resolves at 45.16s.
- Hold the complete CTA for at least 1.2 seconds. No mandatory fade to black.

## Recurring motifs

- Dispatch IDs advance from `SESSION 10` to `ROUTE 06` to `WORKTREE 04` to `READY 01`.
- Thin green routes originate from the real laptop or actual selected pane edges.
- Blue appears only for focus, green only for selection/routing, amber only for completion.
- Jason’s red hoodie remains the warm human counterpoint; do not introduce another red graphic accent.

## Negative prompt

No face cutout, background replacement, cyan-on-purple gradient, glassmorphism,
fake dashboards, fake metrics, glowing HUD circles, particles, equalizer bars,
gradient text, tiny web typography, generic centered SaaS hero, invented product
behavior, or cross-platform publication before matching release artifacts exist.
