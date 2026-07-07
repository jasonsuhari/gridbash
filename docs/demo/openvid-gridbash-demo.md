# OpenVid GridBash Demo

This demo uses the OpenVid workflow from
`https://github.com/CristianOlivera1/openvid`: import a screen recording, apply a
browser or desktop mockup, add smooth zoom points, tune padding and rounded
corners, then export a cinematic demo.

OpenVid is browser-first and does not provide a CLI renderer. The committed
asset is generated locally from original GridBash visuals so it is reproducible
without vendoring OpenVid's PolyForm Noncommercial source or media assets.

## Files

- `docs/assets/gridbash-openvid-demo.mp4` - final 1080p demo video.
- `docs/assets/gridbash-openvid-demo-poster.png` - poster frame for embeds.
- `docs/demo/build-openvid-demo.ps1` - reproducible generator for the committed
  media.
- `docs/demo/openvid-gridbash-demo.html` - local HTML/SVG source scene.
- `docs/demo/capture-openvid-demo.mjs` - Chrome DevTools capture and FFmpeg
  encoding driver.

## Rebuild

```powershell
powershell -ExecutionPolicy Bypass -File docs/demo/build-openvid-demo.ps1
```

The script renders the original GridBash terminal-grid scene in headless Chrome,
captures deterministic frames through the Chrome DevTools Protocol, then encodes
the MP4 with FFmpeg.

## OpenVid Recipe

Use this recipe when recreating or refining the asset inside OpenVid:

- Import `docs/assets/gridbash-openvid-demo.mp4` as the source video.
- Canvas: `16:9`, export quality `1080p (1920x1080) @ 30fps`.
- Mockup: desktop browser or macOS-style frame.
- Background: dark custom gradient with green, cyan, and amber accents.
- Effects: padding `10`, rounded corners `10`, shadows `10`, background blur
  `0`.
- Zoom points:
  - `00:07.0` to `00:10.0`: slight push into the live grid.
  - `00:10.0` to `00:14.5`: focus the selected/broadcast panes.
  - `00:14.5` to end: pull back to the full grid.
- Export: MP4/H.264, 1080p.

## Acceptance Notes

- Shows composer setup, profile/folder selection, pane preview, launch, focused
  pane selection, and selected broadcast.
- Avoids OpenVid source or asset redistribution; only the public OpenVid editing
  workflow is documented.
