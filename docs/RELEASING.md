# Releasing GridBash

This repo has an agent-friendly release path:

1. Land and verify the product change.
2. Create a devlog.
3. Run the `Release` GitHub Actions workflow.
4. For a versioned release, GitHub Actions creates the release commit and tag;
   nightly builds stamp only their ephemeral build workspace.
5. GitHub Actions publishes npm and creates the GitHub release.

The release workflow can be run manually from GitHub Actions, runs nightly from
`main`, and also responds to pushed tags named `v*`.

## One-Time Setup

Prefer npm Trusted Publishing for the `Release` workflow. Configure the root
package and all five native packages on npmjs.com to trust this repository's
`Release` workflow (owner `jasonsuhari`, repository `gridbash`). New package
names must be bootstrapped once with a short-lived npm token before npm can
attach their trusted publisher settings.

After creating a package or receiving one from npm Support, confirm that
`jasonmatthewsuhari` is an owner and configure the trusted publisher before the
first GridBash release:

```sh
npm owner ls gridbash-win32-x64
npm trust github gridbash-win32-x64 \
  --repo jasonsuhari/gridbash \
  --file release.yml \
  --allow-publish
npm trust list gridbash-win32-x64
```

Repeat the trust setup for `gridbash` and every package under `npm/platforms/`.
The trust command requires an interactively authenticated npm account with 2FA.
Use only the workflow filename (`release.yml`), not its full repository path.

The release workflow runs `node npm/scripts/release-preflight.js` before the
native build matrix. It fails early when a package is absent, is not owned by
`jasonmatthewsuhari`, or advertises a different source repository. A package
that npm transferred from its `0.0.1-security` holder is allowed once ownership
is correct; its first GridBash publish replaces the placeholder repository
metadata. The public preflight cannot inspect trusted-publisher settings, so
`npm trust list` remains the authenticated setup check for each new package.

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
2. Select `Run workflow` and leave `channel` set to `release`.
3. Set `version` to `patch`, `minor`, `major`, or an exact version. A plain
   version such as `0.2.0` becomes GitHub's Latest release and npm's `latest`
   dist-tag. A hyphenated prerelease uses npm's `next` dist-tag instead.
4. Optionally set `notes` to a devlog path such as `docs/devlogs/YYYY-MM-DD-title.md`.
5. Run the workflow.

The workflow runs `node npm/scripts/release.js` on `main`. That script creates
and pushes the release commit and `vX.Y.Z` tag. A separate publish job in the
same workflow run then builds Windows x64, Linux x64/arm64, and macOS arm64/x64
native packages, publishes those packages before the platform-neutral npm
launcher, and creates or updates one GitHub release.

Stable releases publish unsigned macOS artifacts until Developer ID signing and
notarization are configured. macOS users may therefore see Gatekeeper warnings.
This packaging limitation no longer prevents Windows and Linux users from
receiving stable releases through the shared GitHub and npm release workflow.

The channel mapping is automatic:

- plain versions such as `0.2.0`: GitHub Latest and npm `latest`
- prereleases such as `0.3.0-beta.1`: GitHub prerelease and npm `next`
- scheduled nightlies: GitHub prerelease and npm `nightly`

Promoting an existing GitHub prerelease manually does not change npm dist-tags.
Run the workflow with a plain stable version to update both release channels.

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

### Registry Publication Incident

When GitHub artifacts exist but npm publication is delayed or blocked by the
registry:

1. Treat npm as unavailable for the affected version or platform. Do not claim
   that `npm install -g gridbash` delivers the GitHub release until it does.
2. Keep the existing release tag immutable. Do not create replacement tags or
   bump versions solely to retry an external registry incident.
3. Put a temporary, factual notice in the website or launch material and pause
   broad promotion. Direct testers may use matching GitHub release artifacts.
4. Record the failing workflow URL and the external support case privately;
   never put credentials or private support correspondence in the repository.
5. After npm confirms resolution, rerun the existing exact-version workflow or
   failed publish job. The publish step is intentionally idempotent and skips
   package versions that are already live.
6. Verify the root launcher and every native package before clearing the notice:

   ```sh
   npm view gridbash version
   npm view gridbash-win32-x64 version
   npm view gridbash-linux-x64 version
   npm view gridbash-linux-arm64 version
   npm view gridbash-darwin-arm64 version
   npm view gridbash-darwin-x64 version
   ```

The GitHub release succeeding does not make a failed npm publish workflow green.
Keep the failure visible until the registry-side work is complete; do not weaken
release checks to improve the status signal.

### Nightlies

The same `Release` workflow runs daily from `main` and supports a manual
`channel: nightly` dispatch. It stamps an immutable version such as
`0.2.0-nightly.20260713.42.gabcdef123456` only in the build workspace, so
nightlies do not create commits or tags on `main`. The workflow skips a nightly
when the same commit already has a published nightly, unless `force_nightly` is
selected.

Install the rolling channel with:

```sh
npm install --global gridbash@nightly
```

Prerelease builds use npm's `next` channel instead:

```sh
npm install --global gridbash@next
```

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
