# Devlogs

Devlogs are human-readable source notes for releases and article-style updates.

Create one with:

```powershell
npm run devlog -- --title "Short descriptive title"
```

The release script copies the chosen devlog to `docs/releases/vX.Y.Z.md`, which the GitHub release workflow uses as release notes.

Keep entries concise and concrete. Write for someone deciding whether to upgrade or skim the change history.
