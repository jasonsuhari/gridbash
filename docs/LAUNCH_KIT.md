# GridBash Launch Kit

Use this kit only after the publication gate in [`OUTREACH.md`](OUTREACH.md)
passes. Adapt the copy to the community; do not post the same announcement
everywhere verbatim.

## Positioning

### One-line pitch

GridBash is a cross-platform terminal workspace for running CLI coding agents
side by side.

### Short pitch

GridBash puts real PTY-backed Codex, Claude, Gemini, Aider, OpenCode, Goose,
Amp, Cursor, Copilot, shell, and custom-command sessions into one selectable
grid. Route a prompt to one pane or a selected set, and give parallel jobs
isolated repo-local git worktrees.

### Proof points

- Windows x64, Linux x64/arm64, and macOS arm64/x64 release artifacts.
- Real PTYs rather than simulated agent output.
- Up to 100 panes across tabbed grids.
- Focused, selected-pane, and grid-wide input routing.
- Optional repo-local git worktree per pane.
- MIT licensed and implemented in Rust.

### Honest constraints

- GridBash is single-process today; closing it closes its child agents.
- Session resume relaunches panes with saved context. It does not reattach live
  processes or replay commands.
- macOS artifacts may show Gatekeeper warnings until signing and notarization
  are configured.

## Publication Gate

Immediately before publishing:

```sh
npm view gridbash version
```

Compare that value with the latest stable GitHub release and verify the exact
install command on the advertised platform. If npm trails GitHub because of a
registry incident, pause broad promotion and use the matching GitHub artifact
only for direct, clearly labeled testing outreach.

## Links And Assets

- Repository: https://github.com/jasonsuhari/gridbash
- Website: https://jasonsuhari.github.io/gridbash/
- npm: https://www.npmjs.com/package/gridbash
- Releases: https://github.com/jasonsuhari/gridbash/releases/latest
- Current product walkthrough: `docs/assets/gridbash-openvid-demo.mp4`
- Walkthrough poster: `docs/assets/gridbash-openvid-demo-poster.png`
- Social preview: `docs/assets/gridbash-social-preview.png`

The older launch teaser ends with a Windows-only card and should not be used for
current cross-platform outreach.

## Primary Workflow Demo

Show one implementation/review/test loop:

1. Launch three Codex panes with worktree isolation.
2. Give one pane an implementation task.
3. Give a second pane a review task.
4. Give a third pane the focused test or documentation task.
5. Show selected-pane follow-up routing.
6. End on the resulting workflow, not a feature list.

Recommended command after the publication gate passes:

```sh
npm install -g gridbash
gridbash 1x3 --profile codex --worktrees
```

## Show HN

### Title

```text
Show HN: GridBash – run CLI coding agents in one selectable terminal workspace
```

### First comment

```text
I kept ending up with a pile of terminal windows while one coding agent
implemented, another reviewed, and a third ran tests. I built GridBash, a Rust
TUI that keeps real PTY sessions in one grid and lets me decide exactly which
panes receive each prompt.

The workflow I care about most is parallel work without accidental cross-pane
input. GridBash can also start every pane in a separate repo-local git
worktree, so implementation, review, tests, and docs stay isolated while they
remain visible together.

It runs on Windows, Linux, and macOS and is MIT licensed:

    npm install -g gridbash
    gridbash 1x3 --profile codex --worktrees

Source and demo: https://github.com/jasonsuhari/gridbash

I would especially value feedback on the selected-pane routing model and what
you would need before trusting it with a daily multi-agent workflow.
```

Submit the repository URL, remain available for the first several hours, and
never ask people to upvote or seed comments.

## Social Posts

Upload the walkthrough video natively. Each post should demonstrate one
workflow and ask one question.

### X Or Bluesky

```text
Running an implementation agent, a reviewer, and a test agent used to mean
three terminal windows.

GridBash keeps their real CLI sessions in one terminal workspace, routes a
prompt only to the panes I select, and can put every job in its own git
worktree.

Open source for Windows, Linux, and macOS:
https://github.com/jasonsuhari/gridbash

Where does your multi-agent terminal workflow break down?
```

### LinkedIn

```text
I built the terminal workflow I wanted for parallel coding agents.

GridBash keeps Codex, Claude, Gemini, Aider, and other real CLI sessions in one
selectable workspace. I can route a prompt to one pane or a chosen set, keep
every session visible, and isolate parallel jobs in repo-local git worktrees.

It is a cross-platform Rust TUI and is open source under the MIT License:
https://github.com/jasonsuhari/gridbash

I am looking for developers who already run several agent sessions. Which
setup, visibility, or safety problem should I test next?
```

## Agent-Specific Communities

Use the terminology and exact profile for that community.

```text
I often run one <AGENT> session for implementation, another for review, and a
third for tests or docs. GridBash keeps those real sessions visible in one
terminal workspace and routes follow-ups only to the selected panes.

Each pane can also start in an isolated git worktree:

    gridbash 1x3 --profile <PROFILE> --worktrees

It is MIT licensed and runs on Windows, Linux, and macOS:
https://github.com/jasonsuhari/gridbash

If you already run multiple <AGENT> sessions, I would value feedback on the
selection and worktree workflow.
```

Read current community rules first. Prefer a native demo and useful workflow
explanation over a bare repository link.

## Rust And Terminal Communities

Do not lead with a generic AI-tool announcement. Teach the implementation:

```text
I built a cross-platform Rust TUI that hosts many real PTYs in one process. The
hard parts were containing mouse selection within a pane, keeping modeless
input routing predictable, and redrawing many live terminals without turning
the UI into a dashboard abstraction.

The implementation uses Ratatui and portable-pty, with repo-local git
worktrees as the isolation boundary for parallel coding-agent jobs.

Architecture, demo, and source:
https://github.com/jasonsuhari/gridbash

I would value technical feedback on <ONE SPECIFIC AREA>.
```

The longer article lives at
[`docs/articles/building-a-windows-pty-grid-in-rust.md`](articles/building-a-windows-pty-grid-in-rust.md).
Update its title in a future editorial pass if the article is expanded from its
Windows/ConPTY origin story into the complete cross-platform architecture.

## Contributor Recruitment

Keep three to five issues genuinely ready before recruiting.

### This Week In Rust Project Update

```text
GridBash is a cross-platform Rust TUI for running CLI coding agents in a
selectable PTY grid, with optional git-worktree isolation. The current focus is
first-run reliability, terminal compatibility, and managed multi-agent
workflows. Source and contributor guide:
https://github.com/jasonsuhari/gridbash
```

### Call For Participation

```text
GridBash – <ISSUE TITLE>: <ONE-SENTENCE OUTCOME>. The issue names the relevant
files, acceptance checks, and focused validation command. Maintainer
orientation is available: <ISSUE URL>
```

### Direct Contributor Invitation

```text
You mentioned an interest in <RUST/TUI/TERMINAL AREA>. GridBash has a scoped
issue in that area with the likely files and validation written down:
<ISSUE URL>

No pressure to take it, but I am happy to provide orientation if the task is a
fit.
```

## Product Hunt

Use Product Hunt only after the project has several activated users, reliable
installation, and at least one concrete user story.

Name:

```text
GridBash
```

Tagline:

```text
Run CLI coding agents in one terminal workspace
```

Description:

```text
A cross-platform Rust TUI for running Codex, Claude, Gemini, Aider, and other
CLI agents side by side. Route prompts to selected panes and isolate parallel
jobs with repo-local git worktrees.
```

Suggested topics: Developer Tools, Open Source, Artificial Intelligence.

## Response Bank

### Why not tmux?

```text
tmux is excellent. GridBash is narrower: it is built around selecting agent
panes, routing the same input intentionally, launching common agent profiles,
and creating repo-local worktrees. If tmux already fits your workflow, keep it.
```

### Is this an agent framework?

```text
It is terminal-level orchestration, not a replacement agent protocol. GridBash
launches and routes input among independent CLI agents while preserving their
native interfaces.
```

### Does closing it kill the agents?

```text
Yes. GridBash is intentionally single-process today, so closing it closes its
children. Daemon-backed detach and reattach is the next major architecture
boundary.
```

### Which platforms are supported?

```text
GridBash builds native artifacts for Windows x64, Linux x64/arm64, and macOS
arm64/x64. Check the latest GitHub release and npm version badge before
installing; the registry can temporarily trail a GitHub release.
```

## Publication Sequence

Do not publish every item on the same day:

1. Clear the publication gate and invite a small set of direct testers.
2. Publish one native workflow video on the strongest existing social channel.
3. Run Show HN and remain available to answer questions.
4. Publish the technical Rust or terminal article.
5. Submit one contributor task to This Week in Rust.
6. Share a user-driven fix or outside contribution.
7. Consider Product Hunt only after installation and retention evidence are
   strong.

## Tracking

Use source-specific links when useful:

```text
https://jasonsuhari.github.io/gridbash/?utm_source=hackernews&utm_medium=community&utm_campaign=workflow-launch
https://jasonsuhari.github.io/gridbash/?utm_source=linkedin&utm_medium=social&utm_campaign=workflow-launch
https://jasonsuhari.github.io/gridbash/?utm_source=thisweekinrust&utm_medium=community&utm_campaign=contributors
```

Record trials, activations, returning users, useful conversations, issues
claimed, and merged outside pull requests in the campaign log described in
[`OUTREACH.md`](OUTREACH.md).

## Before Every Post

- Clear the publication gate.
- Read the community's current self-promotion rules.
- Show one workflow rather than a feature montage.
- Upload video natively where possible.
- State current limitations plainly.
- Ask for product or architecture feedback, never coordinated votes.
- Be ready to answer questions and ship small fixes while attention is active.
