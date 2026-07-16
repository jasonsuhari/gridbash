# Community Operations

This document records how GridBash keeps contribution infrastructure healthy.
Live repository settings and issue lists are the source of truth; avoid copying
issue numbers or release status into this file because they become stale.

## Public Community Surfaces

- `README.md` explains the product, supported platforms, quick start, and ways
  to participate.
- `CONTRIBUTING.md` covers development setup, validation, issue selection, pull
  requests, and DCO sign-offs.
- `CODE_OF_CONDUCT.md`, `SECURITY.md`, and `SUPPORT.md` route conduct, private
  vulnerability reports, and usage questions.
- GitHub Discussions holds workflow examples, open-ended questions, and early
  design conversations.
- GitHub Issues holds reproducible bugs and accepted, scoped work.
- [`OUTREACH.md`](OUTREACH.md) defines the user and contributor growth loop.

The repository description should lead with the current product:

```text
Cross-platform terminal workspace for running CLI coding agents side by side.
```

Useful topics include `rust`, `tui`, `terminal`, `multiplexer`,
`developer-tools`, `ai-agents`, `coding-agents`, `codex`, `claude`, `gemini`,
`ratatui`, `pty`, `worktrees`, `windows`, `linux`, and `macos`.

## Conversation Routing

- **Discussion:** workflow questions, show and tell, polls, and ideas that still
  need discovery.
- **Issue:** a concrete bug, accepted feature, documentation change, test, or
  maintenance task.
- **Pull request:** an implementation with a reviewable outcome.
- **Private report:** a security vulnerability as described in `SECURITY.md`.

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

When a Discussion reaches a concrete outcome, create an issue that captures the
decision and links back to the conversation.

## Labels

Keep type, status, area, platform, and priority labels small enough to remain
useful. Two labels are especially important for outside contributors:

- `help wanted` means the task is accepted and outside contributions are
  actively welcome.
- `good first issue` is a stricter subset with a narrow outcome, named file
  area, focused validation, and no hidden architecture decision.

Do not label a broad roadmap idea as a good first issue merely to attract help.
Keep three to five contributor-ready issues active and remove the labels when a
task becomes stale, blocked, assigned, or underspecified.

## Contributor-Ready Issue Checklist

Every promoted issue should include:

- the user or maintainer problem;
- one concrete outcome;
- the likely files or modules involved;
- acceptance checks;
- the narrowest useful validation command;
- expected size or complexity;
- a clear invitation to comment for orientation.

The maintainer should acknowledge contributor questions and claims within one
business day, confirm scope before substantial work begins, and distinguish
required review changes from optional follow-up ideas.

## Contribution Rights

GridBash uses DCO sign-offs for routine contributions. `CLA.md` is an inactive
template, not an additional current requirement. Do not require both a DCO and
a separate CLA without a clear legal reason.

## Repository Settings Audit

Review these settings after major GitHub or release-workflow changes:

- Issues and Discussions are enabled.
- The repository homepage and social preview are current.
- Delete-branch-on-merge is enabled.
- Secret scanning and push protection are enabled where available.
- `main` blocks force pushes and requires the current CI and DCO checks.
- Required check names match the current cross-platform CI matrix.
- Private vulnerability reporting is enabled.

Do not preserve old required-check names such as a Windows-only CI job after the
workflow becomes cross-platform.

## Monthly Maintenance

Once per month:

1. Review `good first issue` and `help wanted` for stale or blocked work.
2. Check unanswered Discussions and contributor questions.
3. Confirm README, website, roadmap, and launch copy agree on platforms and
   installation status.
4. Recognize merged outside contributions in release notes.
5. Review the campaign log in [`OUTREACH.md`](OUTREACH.md) and continue only the
   channels producing activated users, useful feedback, or contributors.
