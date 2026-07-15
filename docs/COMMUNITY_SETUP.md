# Community Setup Notes

This document records the contribution infrastructure added to GridBash, plus the live GitHub settings and follow-up maintainer actions.

## Research Basis

The setup follows GitHub's community health model: a public repository is easier to evaluate when it has visible README, license, contributing, code of conduct, support, security, and issue template files in supported locations.

GitHub surfaces `CONTRIBUTING.md` when people open issues or pull requests, and also shows it in repository navigation. Issue and pull request templates reduce maintainer back-and-forth by asking for the details reviewers need before the conversation starts.

For contribution rights, this repo starts with DCO sign-offs because they are lightweight and familiar in open source. A separate CLA is kept as an optional template because CLAs can be useful for organizations, but they add more contributor friction and should be reviewed before enforcement.

## What Is Now In The Repository

- `CONTRIBUTING.md` for contributor onboarding, setup, validation, issue guidance, pull request expectations, and DCO instructions.
- `CODE_OF_CONDUCT.md` for community expectations and enforcement.
- `SECURITY.md` for private vulnerability reporting.
- `SUPPORT.md` for usage help routing.
- `GOVERNANCE.md` for maintainer-led project governance.
- `DCO.md` plus `.github/workflows/dco.yml` to require commit sign-offs on pull requests.
- `CLA.md` as a CLA template if separate CLA enforcement is later needed.
- `.github/ISSUE_TEMPLATE/` issue forms for bugs, features, and questions.
- `.github/pull_request_template.md` for consistent review context.
- `.github/CODEOWNERS` to route reviews to the maintainer when branch protection is enabled.
- `docs/assets/gridbash-social-preview.png` as the repository social preview source asset.

## GitHub Repository Settings

Configured values:

- Issues: enabled.
- Projects: enabled.
- Discussions: enabled.
- Wiki: enabled.
- Delete branch on merge: enabled.
- Secret scanning: enabled.
- Secret scanning push protection: enabled.
- Description: `Local workspace for running and coordinating CLI coding agents in parallel.`
- Homepage: blank until npm metadata is corrected or a dedicated landing page exists.
- Topics: `rust`, `tui`, `terminal`, `agent-workspace`, `cli`, `developer-tools`, `ai-agents`, `coding-agents`, `codex`, `claude`, `ratatui`, `orchestration`, `npm-package`, `open-source`.

Settings that still require browser or elevated-token access:

- Upload `docs/assets/gridbash-social-preview.png` in Settings > General > Social preview.
- Create the modern GitHub Project after refreshing the GitHub CLI token with `gh auth refresh --hostname github.com -s project,read:project`.
- Enable private vulnerability reporting in Settings > Code security and analysis after `SECURITY.md` is on the default branch.
- Protect `main` after the community files land:
  - Require the `CI / windows` check.
  - Require the `DCO / Signed-off-by` check.
  - Require pull request review before merging.
  - Require conversation resolution before merging.
  - Block force pushes.

## Labels

Default labels are preserved:

- `bug`
- `documentation`
- `duplicate`
- `enhancement`
- `good first issue`
- `help wanted`
- `invalid`
- `question`
- `wontfix`

Additional labels:

- `priority:p0`, `priority:p1`, `priority:p2`, `priority:p3`
- `status:needs-triage`, `status:accepted`, `status:blocked`, `status:needs-repro`
- `area:pty`, `area:tui`, `area:profiles`, `area:composer`, `area:config`, `area:packaging`, `area:docs`, `area:architecture`
- `platform:windows`
- `type:test`, `type:design`, `type:maintenance`

New issues are classified by `.github/workflows/issue-labeler.yml`. The
automation adds only missing type, triage-status, area, and Windows-platform
labels; priorities and resolution labels remain maintainer decisions.

## Milestones

- `v0.2 Nostromo`: public launch polish.
- `v0.3 Nebuchadnezzar`: agent orchestration workflows.
- `v0.4 Holodeck`: TUI/runtime experience.
- `v1.0 Zion`: stable Windows release.
- `v2.0 Morpheus`: daemon and reattach architecture.

Milestones intentionally have no due dates.

## Starter Issues

- `#1 docs: add screenshot and quick-start flow to README`
- `#2 chore: publish current npm package metadata under this repo`
- `#3 docs: add release checklist for Windows binary and npm tarball`
- `#4 test: verify first-run onboarding on clean Windows profile`
- `#5 feat: improve profile detection diagnostics`
- `#6 feat: polish startup picker controls`
- `#7 test: add startup picker preview coverage`
- `#8 feat: persist settings screen controls`
- `#9 feat: add in-app help and legend overlay`
- `#10 docs: define v1.0 acceptance checklist`
- `#11 design: outline daemon detach and reattach architecture`

## Project Setup

Create a modern GitHub Project named `GridBash Roadmap` after the token has Project scopes:

```powershell
gh auth refresh --hostname github.com -s project,read:project
gh project create --owner jasonsuhari --title "GridBash Roadmap"
```

Add fields:

- `Priority`: `P0`, `P1`, `P2`, `P3`
- `Release`: `v0.2 Nostromo`, `v0.3 Nebuchadnezzar`, `v0.4 Holodeck`, `v1.0 Zion`, `v2.0 Morpheus`
- `Area`: `Docs`, `Packaging`, `TUI`, `PTY`, `Profiles`, `Composer`, `Config`, `Architecture`
- `Size`: `S`, `M`, `L`

Recommended views:

- `Roadmap`
- `Board by Status`
- `Table by Release`
- `Good First Issues`

Add issues `#1` through `#11` to the project.

## Community Surfaces

- Discussions are enabled.
- The seed discussion is `Welcome to the GridBash roadmap`.
- Wiki is enabled. If `gridbash.wiki.git` does not exist yet, create the first page in the GitHub UI, then push a Home page that links to README, Contributing, Roadmap, Issues, Discussions, and the Project.

## Branch Protection

After this branch is merged, protect `main`:

```text
Require pull request before merging
Require approvals: 1
Require conversation resolution before merging
Require status checks: CI / windows, DCO / Signed-off-by
Block force pushes
```

## Social Preview

Upload `docs/assets/gridbash-social-preview.png` in repository Settings > General > Social preview.

The source asset is 1280x640 PNG and intentionally avoids third-party logos or copyrighted characters.

## Private Vulnerability Reporting

After `SECURITY.md` reaches the default branch, enable private vulnerability reporting in Settings > Code security and analysis.

## CLA Activation

GridBash currently uses DCO sign-offs because they are lighter for contributors than a separate CLA.

If a separate CLA becomes necessary:

1. Have qualified counsel review `CLA.md`.
2. Publish the final CLA text somewhere stable, such as a GitHub Gist used only for the CLA.
3. Link the repository or organization to a CLA tool such as CLA Assistant.
4. Add the CLA status check to branch protection.
5. Update `CONTRIBUTING.md` and `.github/pull_request_template.md` to state that CLA signing is mandatory.

Do not require both DCO and a CLA unless there is a clear legal reason. That adds friction for new contributors.

## Good First Issue Practice

A good first issue should have:

- A single clear outcome.
- Reproduction steps or an exact file area.
- A suggested validation command.
- No hidden product decision.
- No expected knowledge of GridBash internals beyond the files named in the issue.
