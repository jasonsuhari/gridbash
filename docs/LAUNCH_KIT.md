# GridBash Launch Kit

This is the reusable publication kit for GridBash. Copy should be adapted to
the conversation and community instead of posted everywhere verbatim.

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
- Up to 100 panes in one process
- Managed Claude/Codex auth selection and usage visibility
- Input routing to one, selected, or all panes
- Optional repo-local git worktree per pane
- Built-in profiles for Codex, Claude, Gemini, Aider, OpenCode, Goose, Amp,
  Cursor, and Copilot
- Open source under the MIT License

### Honest constraint

V1 is intentionally local and single-process; closing GridBash closes its child
agents. Raw terminal grids remain available, but managed auth only applies to
agents GridBash launches.

## Links and assets

- Repository: https://github.com/jasonsuhari/gridbash
- Website: https://jasonsuhari.github.io/gridbash/
- npm: https://www.npmjs.com/package/gridbash
- Launch teaser: https://github.com/jasonsuhari/gridbash/blob/main/docs/assets/gridbash-launch-teaser.mp4
- Teaser poster: `docs/assets/gridbash-launch-teaser-poster.png`
- Product walkthrough: https://github.com/jasonsuhari/gridbash/blob/main/docs/assets/gridbash-openvid-demo.mp4
- Social preview: `docs/assets/gridbash-social-preview.png`

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

The workflow I care about most is parallel agent work without accidental
cross-pane input. A pane can also start in its own repo-local git worktree, so
implementation, review, tests, and docs can run in isolation while remaining
visible together.

It is open source and cross-platform:

    npm install -g gridbash
    gridbash 2x3 --profile codex --worktrees

I would especially appreciate feedback on the input-routing model and what you
would expect from detach/reattach support.
```

Submit the repository URL, remain available to answer questions, and do not ask
people to upvote or seed comments.

## Reddit

Read the current rules of each subreddit before posting. Use only the version
that matches the community and stay in the thread to answer questions.

### r/rust

**Title**

```text
I built a Rust TUI for running multiple coding-agent PTYs in one Windows terminal
```

**Body**

```text
I wanted one terminal surface for several CLI coding agents without hiding the
process behind a web dashboard. GridBash uses Ratatui plus Windows ConPTY-backed
sessions, routes input to one or selected panes, and can start every pane in a
separate git worktree.

The tricky parts were keeping redraws cheap with many live panes, containing
mouse selection inside a pane, and making normal terminal input coexist with
modeless routing shortcuts.

The project is MIT licensed and installable with `npm install -g gridbash` on
Windows, Linux, and macOS. Source and a 13-second demo:
https://github.com/jasonsuhari/gridbash

I would love feedback on the terminal architecture and where the Rust side
could be simplified.
```

### r/commandline

**Title**

```text
GridBash: one local workspace for Codex, Claude, Gemini, and other CLI agents
```

**Body**

```text
I built GridBash because parallel coding-agent work kept turning into a pile of
disconnected terminals. It launches real PTY sessions in one managed workspace,
lets you select exactly which panes receive a command, manages Claude/Codex auth,
and can isolate panes in repo-local git worktrees.

Quickstart:

    npm install -g gridbash
    gridbash 2x3 --profile codex --worktrees

Demo and source: https://github.com/jasonsuhari/gridbash

I am most interested in feedback from people already using tmux, Windows
Terminal, or several CLI agents at once. What would make this fit your workflow?
```

### Agent-specific communities

**Title template**

```text
I built a local workspace for running multiple <AGENT> sessions in parallel
```

**Body template**

```text
I often run one <AGENT> session for implementation, another for review, and a
third for tests or docs. GridBash launches those real CLI sessions into one
managed workspace and lets me route prompts only to the selected panes.

The built-in <AGENT> profile launches a grid directly, and `--worktrees` gives
each pane an isolated checkout:

    gridbash 2x3 --profile <PROFILE> --worktrees

It is MIT licensed and ships for Windows, Linux, and macOS:
https://github.com/jasonsuhari/gridbash

If you use multiple <AGENT> sessions, I would value feedback on the selection
and worktree workflow.
```

## Product Hunt

### Name

```text
GridBash
```

### Tagline

```text
Your local workspace for parallel coding agents
```

### Description

```text
An open-source local workspace for launching, authenticating, isolating,
monitoring, and coordinating Codex, Claude, Gemini, Aider, and other CLI agents
in parallel while preserving their native terminal interfaces.
```

### First comment

```text
I built GridBash after parallel coding-agent work turned my desktop into a pile
of terminals. I wanted the sessions to stay real and visible, but I also wanted
one reliable way to decide which agents receive each prompt.

GridBash runs PTY-backed sessions in a selectable grid. You can focus one pane,
select several, broadcast when appropriate, and start each pane in its own git
worktree. It is open source, MIT licensed, and available for Windows, Linux,
and macOS through npm.

I am here all day and would love blunt feedback, especially from developers who
already run several CLI agents at once.
```

Suggested topics: Developer Tools, Open Source, Artificial Intelligence.

## Social posts

Attach `gridbash-launch-teaser.mp4` directly instead of relying on a link
preview. Put the repository link in the post or first reply according to the
platform's current link treatment.

### X / Bluesky

```text
Running six coding agents used to mean six terminal windows.

So I built GridBash: an open-source local workspace for launching and
coordinating Codex, Claude, Gemini, Aider, and other CLI agents in parallel.

Each pane can even get its own git worktree.

Open source: https://github.com/jasonsuhari/gridbash
```

### LinkedIn

```text
I built the terminal workflow I wanted for parallel coding agents.

GridBash launches Codex, Claude, Gemini, Aider, and other CLI tools into one
managed PTY-backed workspace. I can choose auth and worktree isolation up front,
route a prompt to one pane or selected panes, and keep every session visible.

It is a Rust TUI, MIT licensed, and available for Windows, Linux, and macOS:
https://github.com/jasonsuhari/gridbash

The most useful feedback now is from developers already juggling multiple agent
sessions: where does your workflow break down?
```

### Short Discord post

```text
I made GridBash, an open-source local workspace for running and coordinating
multiple CLI coding agents in parallel. It supports managed auth, selected-pane
input, and optional git worktree isolation. Demo + source: https://github.com/jasonsuhari/gridbash
```

## Technical article outline

The complete publication draft lives at
[`docs/articles/building-a-windows-pty-grid-in-rust.md`](articles/building-a-windows-pty-grid-in-rust.md).

**Title:** Building a Windows PTY grid for coding agents in Rust

1. Why multiple coding agents create a terminal coordination problem
2. Why GridBash preserves real PTYs instead of wrapping agents behind an API
3. ConPTY lifecycle and terminal emulation constraints
4. Routing ordinary input without introducing modal friction
5. Pane-local mouse selection and redraw performance
6. Git worktrees as the isolation boundary for parallel agents
7. What V1 deliberately does not solve: daemon detach/reattach
8. Architecture diagram, performance numbers, install command, and repository

The article should teach the terminal lessons first and mention GridBash as the
working implementation rather than reading like an advertisement.

## Response bank

### “Why not tmux?”

```text
tmux is excellent. GridBash is not trying to replace a general-purpose terminal
multiplexer: it owns the local agent workflow around launch, auth, usage,
selection, coordination, and repo-local worktrees. Raw shells remain available.
```

### “Why Windows only?”

```text
The published package started with Windows because ConPTY workflows were the
gap I personally had. Cross-platform packaging is active work, but I do not
want to claim a platform until a release artifact is actually available.
```

### “Is this really multi-agent orchestration?”

```text
It is a local agent workspace, not an agent framework or remote control plane.
GridBash launches and coordinates independent CLI agents without hiding their
native interfaces or inventing a shared agent protocol.
```

### “Does closing it kill the agents?”

```text
Yes in V1. GridBash is intentionally single-process today, so closing it closes
its children. Daemon-backed detach/reattach is the major next frontier.
```

## Publication sequence

Do not dump every post on the same day. A practical sequence:

1. Publish Show HN and stay available for the first several hours.
2. Post the technical Rust version the next day if it complies with current
   subreddit rules.
3. Post the workflow version to command-line and agent-specific communities on
   separate days.
4. Publish the short video to social accounts with native upload.
5. Publish the technical article and submit it as a normal HN story, not a
   second Show HN.
6. Schedule Product Hunt for a day when the maker can answer comments throughout
   the Pacific-time launch window.

## Tracking

Use source-specific links when analytics are needed:

```text
https://jasonsuhari.github.io/gridbash/?utm_source=hackernews&utm_medium=community&utm_campaign=launch
https://jasonsuhari.github.io/gridbash/?utm_source=reddit&utm_medium=community&utm_campaign=launch
https://jasonsuhari.github.io/gridbash/?utm_source=producthunt&utm_medium=launch&utm_campaign=launch
https://jasonsuhari.github.io/gridbash/?utm_source=linkedin&utm_medium=social&utm_campaign=launch
```

GitHub stars are a lagging signal. Track qualified questions, successful
installs, npm downloads, issue quality, returning contributors, and which
message caused people to try the product.

## Before every post

- Confirm the install command against the current published release.
- Confirm the advertised platforms against actual downloadable artifacts.
- Read the community's current self-promotion rules.
- Upload the video natively when possible.
- Use a title that states what was built, not “Please support my project.”
- Ask for product or architecture feedback, never coordinated votes.
- Be ready to answer questions and ship small fixes while attention is active.
