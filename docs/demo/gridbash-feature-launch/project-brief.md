# GridBash Feature Launch Video Brief

## Outcome

- Product: GridBash v0.2.0 by Jason Suhari.
- Audience: developers running several CLI coding-agent sessions at once.
- Desired action: understand that GridBash now manages the full parallel-agent
  workspace, then install it or visit the repository.
- Success condition: a muted viewer can name at least four capabilities beyond
  tiling terminals and selective prompt routing after one watch.
- Format: 1920×1080, 30 fps, 96 seconds, landscape feature-launch film.

## Message

- Opening promise: the grid was only the beginning.
- Core proof: real PTY panes, selective input routing, managed worktrees, and up
  to 100 panes.
- New product story: GridBash is a workspace and coordination layer with tabs,
  runtime resize/zoom, activity summaries, lifecycle controls, voice input,
  profiles/auth assignment, manager-goal routing, recovery, and an opt-in local
  agent-control API.
- Distribution story: the root npm package resolves native Windows x64, Linux
  x64/arm64, and macOS arm64/x64 binaries at v0.2.0.
- CTA: `npm install -g gridbash` and `github.com/jasonsuhari/gridbash`.

## Truth boundaries

- Use the checked-in six-pane recording as real proof for selection and routed
  replies; do not generate terminal output.
- UI schematics are explanatory motion graphics based on current source labels,
  not presented as screenshots.
- Session resume restores layout, pane metadata, and bounded recent context by
  launching new terminals. It does not reattach old processes or replay commands.
- Voice dictation never submits Enter automatically.
- The manager reviews awake panes and sends validated targeted follow-ups; it is
  not autonomous hidden agent-to-agent communication.
- The local agent-control API is opt-in, localhost-only, and session-token
  authenticated.
- Usage reporting is best-effort and account identifiers remain masked.
- Do not claim detach/reattach, cloud synchronization, or universal signing.

## Production

- Renderer: HyperFrames 0.7.49 with one deterministic paused GSAP timeline.
- Design: tactile operations field manual derived from the approved original
  cut-paper launch system.
- Product media: `assets/product-demo.mp4` and
  `assets/product-demo-poster.png`.
- Delivery: H.264 MP4, 1920×1080, 30 fps.
- Issue: #244.
