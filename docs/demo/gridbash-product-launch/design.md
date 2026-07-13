# GridBash Product Launch Design

## Visual world

Jason’s real desk becomes the operator station for a coding-agent dispatch room.
The physical camera frame is never replaced by a synthetic backdrop: terminal
routes, focus rails, and real product footage attach to the desk and laptop as
if the GridBash control system is waking up around him.

## Palette

- Canvas: `#0b1118`
- Panel: `#121720`
- Foreground: `#f0f4f9`
- Muted copy: `#8fa2b7`
- GridBash green: `#29d392`
- Focus blue: `#48a5ff`
- Completion amber: `#ffca5c`

Use only these declared hex colors in composition CSS. For alpha, use `rgba()`
values derived from the same palette.

## Typography

- Display voice: `Archivo Black`, 400, tracking `-0.045em`.
- Operational voice: `Cascadia Mono`, 400 and 700, tracking `0.02em` to `0.09em`.
- Captions: Archivo Black for 2-4 word emphasis groups, Cascadia Mono for the
  quieter connecting words.
- Display statements: 112-176 px. Captions: 56-78 px. Operational labels: 24-32 px.

## Frame and material

- Master: 1920x1080, 30 fps.
- Camera plate: 16:9 with 18 px corners and a 2 px operational border when not full frame.
- Product plate: square-on, never perspective tilted, with a deep terminal-window shadow.
- Structural rules: 2-4 px, anchored to the safe edges.
- Depth: real camera atmosphere, product/UI midground, routing labels and captions foreground.
- Add fine deterministic grain to unify the 640x360 camera with 1080p graphics; do not use grain to hide unreadable UI.

## Captured GitHub repo plate

- Source: `capture/github/screenshots/scroll-000.png`, captured at 1920x1080.
- GitHub page: `#FFFFFF`; primary text/nav: `#1F2328`; muted text: `#59636E`;
  link blue: `#0969DA`; action green: `#1F883D`; quiet surface: `#F6F8FA`.
- Type: Mona Sans VF 400/500/600 with GitHub's system monospace for code.
- Components: repository header, star button/count, file table, About panel, topic
  chips, MIT resource link, release badge, and language split.
- Present the screenshot square-on in a framed viewport. Use a slow camera crop and
  an external GridBash-green proof rail; do not redraw GitHub or use an iframe.

## Motion language

- Primary transition: velocity-matched directional push, 0.34-0.46 seconds, `power3.inOut`.
- Accent transition: zoom-through into the product reveal, 0.48 seconds, `expo.inOut`.
- Motion verbs: stamp, route, lock, focus, branch, resolve.
- One ambient mechanism per scene; settled UI remains still.
- First motion begins at 0.18 seconds.

## Sound language

- Music: rhythmic industrial/editorial underscore, approximately 103 BPM.
- Selection: short clean click family with slight pitch variation.
- Routing: narrow tick/relay family.
- Worktree transform: one branch whoosh.
- Product and CTA: related impact family, with the product reveal larger.

## Do

- Keep Jason’s face and delivery human, warm, and legible.
- Let the laptop and real desk motivate terminal graphics.
- Use full product footage for routing proof.
- Burn accurate captions for muted social viewing.
- Hold the install command long enough to copy.

## Do not

- Do not matte Jason out or replace the real room.
- Do not use cyan-on-purple gradients, neon HUDs, particles, glowing orbs, or fake dashboards.
- Do not cover Jason’s face with captions or UI.
- Do not make the camera source the only fine-detail focal point at 1080p.
- Do not publish cross-platform copy before the matching packages exist.
- Do not hard-code a stale star count; the captured 2026-07-13 repo frame proves 46.
