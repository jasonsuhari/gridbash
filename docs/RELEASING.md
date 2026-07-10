# Releasing GridBash

This repo has an agent-friendly release path:

1. Land and verify the product change.
2. Create a devlog.
3. Run the `Release` GitHub Actions workflow.
4. GitHub Actions creates the release commit and tag.
5. GitHub Actions publishes npm and creates the GitHub release.

The release workflow can be run manually from GitHub Actions and also responds
to pushed tags named `v*`.

## One-Time Setup

Prefer npm Trusted Publishing for the `Release` workflow. Configure the package
on npmjs.com to trust this repository's GitHub Actions workflow.

As a fallback, add an npm automation token as a GitHub repository secret:

```text
NPM_TOKEN
```

If `NPM_TOKEN` exists, the workflow uses it. If it does not exist, the workflow
tries npm Trusted Publishing through GitHub Actions OIDC. GitHub release
creation uses the built-in `GITHUB_TOKEN`.

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

## Release From GitHub Actions

After the change is merged to `main`:

1. Open the `Release` workflow in GitHub Actions.
2. Select `Run workflow`.
3. Set `version` to `patch`, `minor`, `major`, or an exact version like `0.2.0`.
4. Optionally set `notes` to a devlog path such as `docs/devlogs/YYYY-MM-DD-title.md`.
5. Run the workflow.

The workflow runs `node npm/scripts/release.js` on `main`. That script creates
and pushes the release commit and `vX.Y.Z` tag. A separate publish job in the
same workflow run then builds Windows x64 plus macOS arm64/x64 native packages,
publishes those packages before the platform-neutral npm launcher, and creates
or updates one GitHub release.

macOS releases are preview-only until real-hardware testing and Developer ID
signing/notarization are complete. Dispatch an exact prerelease such as
`0.2.0-macos.1`; prereleases publish under npm's `next` dist-tag. Stable release
requests fail before creating a tag while this gate is active.

Before creating the release commit, the script fetches origin branch refs and
fails if any unmerged task branches remain under `chore/`, `docs/`, `feat/`,
`fix/`, `refactor/`, or `test/`. Review, merge, or delete those branches before
releasing. Use `--allow-unmerged-branches` only when the branch queue was
explicitly reviewed and the release is intentionally shipping without those
changes.

If publishing fails after the tag exists, rerun the failed publish job after
fixing credentials. The publish job skips npm when that exact package version
is already live and updates an existing GitHub release with `--clobber` assets.
If the whole workflow needs to be dispatched again for an exact version whose
tag already exists, the prepare job skips version preparation and publishes the
existing tag.

Separately, if a `v*` tag is pushed from a local release fallback, the tag push
path in the same workflow publishes npm and creates the GitHub release.

## Local Fallback

Use this only when GitHub Actions is unavailable:

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
- require reviewed/merged origin task branches unless `--allow-unmerged-branches` is passed
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

When the tag reaches GitHub, `.github/workflows/release.yml` builds each native
package, publishes the native packages before `gridbash`, and creates or updates
one GitHub release with all artifacts and the release notes.

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

`node npm/scripts/prepare.js` assembles the native package for the current host.
On macOS it builds `GridBash.app` and the nested Apple Speech helper. On Windows,
close running GridBash windows before a local reinstall; Windows can lock the
currently installed executable and make npm fail with `EBUSY`.

For local testing, use:

```powershell
npm run install:local
```

Do not use `npm install -g .` from worktrees. npm creates a global junction to the worktree that ran it, so a later agent can accidentally make the `gridbash` command launch an older branch.
