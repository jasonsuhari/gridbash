# GridBash Launch Kit

Use this kit only after the publication gate in [`OUTREACH.md`](OUTREACH.md)
passes. Adapt the copy to the community; do not post the same announcement
everywhere verbatim.

## Positioning

### One-line pitch

GridBash is an open-source local workspace for running and coordinating Codex,
Claude, Gemini, and other CLI coding agents in parallel.

### Short pitch

GridBash launches real PTY-backed agents into one managed workspace. Choose the
agent, auth, project, layout, and worktree policy up front, then prompt one
pane, selected panes, or the whole workspace without hiding native agent UIs.

### Proof points

- One-command install: `npm install -g gridbash`
- Real PTY sessions rather than simulated output
- Up to 100 panes across tabbed grids
- Native releases for Windows x64, Linux x64/arm64, and macOS arm64/x64
- Managed Claude/Codex auth selection and usage visibility
- Input routing to one, selected, or all panes
- Optional repo-local git worktree per pane
- Built-in profiles for Codex, Claude, Gemini, Aider, OpenCode, Goose, Amp,
  Cursor, and Copilot
- Open source under the MIT License

### Honest constraints

- By default, closing GridBash closes its child agents. Users can opt into local
  background pane hosts with `keep_terminals_running = true`, then reconnect to
  the same processes from a saved session.
- Background pane hosts allow one attached GridBash client at a time; shared
  multi-client attachment is not supported yet.
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
Show HN: GridBash – a local workspace for parallel CLI coding agents
```

### First comment

```text
I kept ending up with six disconnected agent terminals while comparing Codex,
Claude, and Gemini or running agents against separate tasks. I built GridBash,
a local agent workspace that launches real PTY sessions with explicit auth,
project, layout, and worktree choices, then lets me send input to one, several,
or every pane.

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

GridBash launches Codex, Claude, Gemini, Aider, and other CLI tools into one
managed PTY-backed workspace. I can choose auth and worktree isolation up front,
route a prompt to one pane or selected panes, and keep every session visible.

It is a Rust TUI, MIT licensed, and available for Windows, Linux, and macOS:
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
tmux is excellent. GridBash is not trying to replace a general-purpose terminal
multiplexer: it owns the local agent workflow around launch, auth, usage,
selection, coordination, and repo-local worktrees. Raw shells remain available.
```

### Is this an agent framework?

```text
It is terminal-level orchestration, not a replacement agent protocol. GridBash
launches and routes input among independent CLI agents while preserving their
native interfaces.
```

### Does closing it kill the agents?

```text
By default, yes. If you enable `keep_terminals_running`, each pane runs through
a local authenticated background host and a saved session can reconnect to the
same process later. Each host still accepts only one GridBash client at a time.
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
