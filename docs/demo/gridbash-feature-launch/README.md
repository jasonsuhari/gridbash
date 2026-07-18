# GridBash Feature Launch

This HyperFrames composition builds the 96-second, 1920×1080 follow-up launch
film for GridBash. It preserves the approved tactile cut-paper world while
introducing the major current product capabilities omitted from the original
launch film.

## Production files

- `project-brief.md` records the audience, message, and factual boundaries.
- `feature-inventory.md` maps current features to film chapters.
- `design.md` defines the palette, typography, and physical visual world.
- `.hyperframes/expanded-prompt.md` contains the full scene-level direction.
- `scene-plan.md` records timing, proof, and transitions.
- `scripts/build_soundtrack.py` generates the deterministic original sound bed.
- `index.html` is the deterministic HyperFrames composition.

## Product assets

`assets/product-demo.mp4` and `assets/product-demo-poster.png` are the approved
real six-pane GridBash capture from the first launch production. All other UI
representations are clearly art-directed operational schematics based on the
current docs and source labels.

## Validate and render

Requires Node.js 22+, Python with NumPy, FFmpeg, FFprobe, Chrome, and
HyperFrames 0.7.49.

```powershell
python scripts/build_soundtrack.py
npm run lint
npm run validate
npm run inspect
npm run render
```

The final distributable is
`../../assets/gridbash-feature-launch-video.mp4`.
