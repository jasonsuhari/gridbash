# GridBash Product Launch

This HyperFrames composition builds the 1920x1080 talking-head launch film for
GridBash. Jason's real camera footage remains intact while real product footage,
captions, operational graphics, music, and semantic SFX build around it.

## Production files

- `project-brief.md` records verified product claims and release gates.
- `design.md` defines the palette, typography, and desk-as-operator-station world.
- `.hyperframes/expanded-prompt.md` contains the scene-level production breakdown.
- `scene-plan.md`, `music-map.md`, and `approval-log.md` record timing decisions.
- `scripts/build_talking_head.py` extracts the approved takes, rebuilds clean voice
  from the original source, and remaps word-level captions.
- `scripts/record_gridbash_demo.py` records the real six-pane GridBash/Codex proof
  in a disposable repository, including isolated worktrees and subset routing.
- `index.html` is the deterministic HyperFrames composition.

## Local source assets

The raw camera recording, generated take videos, clean voice WAV, soundtrack,
and SFX downloads are intentionally ignored. Their source/licensing records live
beside the asset directories. The final rendered MP4 is the distributable work.

Run the talking-head build after placing the recording at
`raw/jason-talking-head.mp4` and its Whisper JSON at
`transcripts/jason-talking-head.json`:

```powershell
npm run build:talking-head
```

## Validate and render

Requires Node.js 22+, FFmpeg, FFprobe, Chrome, and HyperFrames 0.7.49.

```powershell
npm run check
npm run render
```

The final distributable is `../../assets/gridbash-product-launch-video.mp4`.
The public cross-platform cut must not ship until Windows, Linux, and macOS
package artifacts are verified against the release being promoted.
