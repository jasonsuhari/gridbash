# Releasing GridBash

This repo has an agent-friendly release path:

1. Land and verify the product change.
2. Create a devlog.
3. Run the release script.
4. Push the generated tag.
5. GitHub Actions publishes npm and creates the GitHub release.

The release workflow runs only for tags named `v*`.

## One-Time Setup

Add an npm automation token as a GitHub repository secret:

```text
NPM_TOKEN
```

The release workflow uses that token to publish `gridbash` to npm. GitHub release creation uses the built-in `GITHUB_TOKEN`.

## Create A Devlog

Generate a new draft:

```powershell
npm run devlog -- --title "Startup grid picker"
```

Fill in the generated file under `docs/devlogs/`. Keep it factual:

- what changed
- why it matters
- what was validated
- any known risk

## Release From Main

After the change is merged and the working tree is clean:

```powershell
npm run release -- patch --notes docs/devlogs/YYYY-MM-DD-title.md --push --yes
```

Use `minor` or `major` instead of `patch` when appropriate. You can also pass an exact version:

```powershell
npm run release -- 0.2.0 --notes docs/devlogs/YYYY-MM-DD-title.md --push --yes
```

The script will:

- require a clean working tree
- require `main` or `master` unless `--allow-branch` is passed
- bump `package.json` and `Cargo.toml`
- update `Cargo.lock`
- copy the devlog to `docs/releases/vX.Y.Z.md`
- run `cargo fmt --check`
- run `cargo clippy -- -D warnings`
- run `cargo test`
- run `node npm/scripts/prepare.js`
- run `npm pack --dry-run`
- commit `chore: release vX.Y.Z`
- create tag `vX.Y.Z`
- push the commit and tag when `--push --yes` is passed

When the tag reaches GitHub, `.github/workflows/release.yml` builds the Windows package, publishes npm, and creates a GitHub release with the release notes.

## Local Release Without Push

To create the release commit and tag locally without pushing:

```powershell
npm run release -- patch --notes docs/devlogs/YYYY-MM-DD-title.md
```

Delete them manually if you were only experimenting:

```powershell
git tag -d vX.Y.Z
git reset --hard HEAD~1
```

Only do that for a local release experiment that has not been pushed.

## Common Blocker

`node npm/scripts/prepare.js` copies the freshly built exe into `npm/bin/win32-x64/gridbash.exe`. Close any running GridBash window before releasing, otherwise Windows can lock the target exe and the copy step will fail with `EBUSY`.
