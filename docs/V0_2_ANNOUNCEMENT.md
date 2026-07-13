# GridBash v0.2 announcement kit

## Launch status

- Live preview: https://github.com/jasonsuhari/gridbash/releases/tag/v0.2.0-macos.1
- Verified builds: Windows x64, Linux x64/arm64, macOS arm64/x64
- Nightly channel: active from current `main` at 08:17 UTC
- npm `next`: do not advertise the install command until all six packages pass
  the one-time registry authentication bootstrap tracked in #192

## Hero line

> Your agents do not need more windows. They need a command center.

## X / Bluesky

```text
Your agents don't need more windows. They need a command center.

GridBash v0.2 preview:
⚡ manager orchestration across the grid
🧠 isolated Codex memory per pane
🖥 Windows + Linux + macOS builds
🌙 nightly releases from main

Deploy the squad: https://github.com/jasonsuhari/gridbash
```

### First reply

```text
Grab the v0.2 cross-platform preview and all six build artifacts:
https://github.com/jasonsuhari/gridbash/releases/tag/v0.2.0-macos.1

The macOS builds are preview artifacts and are not signed or notarized yet.
```

## GitHub / LinkedIn / long-form post

```text
GridBash v0.2 preview is live.

Parallel coding agents should feel like a command center, not a desktop
accident. This release moves GridBash beyond tiling terminals and into real
grid-wide coordination.

⚡ Give the grid one goal. The manager reads the live panes, sends targeted
commands, tracks what was actually written, and retries only unfinished work.

🧠 Every Codex pane gets its own persistent SQLite lane, so goals, memories,
and thread relationships stop colliding while auth, config, skills, and rollout
history stay shared.

🖥 The same release now ships native builds for Windows x64, Linux x64/arm64,
and macOS arm64/x64.

And the rest of the grid got sharper: tabs, bounded session resume, a visual
resizer, pane-local scrollback, activity summaries, keyboard-first Pane
Activity controls, voice input, durable themes, and safer terminal I/O.

🌙 Nightly builds now cut automatically from main, so the command center keeps
moving.

Grab the cross-platform preview:
https://github.com/jasonsuhari/gridbash/releases/tag/v0.2.0-macos.1

Run the squad. Break it. Tell me where the command center needs more firepower.
```

## Discord / community post

```text
GridBash v0.2 preview is live. One terminal, a whole squad:

• grid-wide manager orchestration
• isolated Codex memory per pane
• Windows, Linux, and macOS builds
• automatic nightlies from main

Plus tabs, resume, visual resize, scrollback, voice, and a much tougher I/O
path. Grab the preview: https://github.com/jasonsuhari/gridbash/releases/tag/v0.2.0-macos.1
```

## After npm `next` is verified

Only after all six npm packages and their `next` dist-tags are confirmed, add:

```text
npm install --global gridbash@next
```

Until then, link the GitHub prerelease directly.

## Suggested media

- Attach `docs/assets/gridbash-product-launch-video.mp4` natively when the
  platform supports video uploads.
- Use `docs/assets/gridbash-social-preview.png` when a static image is required.
- Put the exact release link in the first reply if the platform suppresses
  posts with outbound links.
