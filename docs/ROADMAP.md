# Roadmap

GridBash is developed in public, but the roadmap describes direction rather
than a promise of dates. Shipped behavior is documented in the
[reference](REFERENCE.md), and accepted work is tracked in
[GitHub Issues](https://github.com/jasonsuhari/gridbash/issues).

## Shipped Foundation

- Real PTY-backed panes on Windows, Linux, and macOS.
- Focused, selected-pane, and grid-wide input routing.
- Agent and shell launch profiles.
- Repo-local git worktree isolation.
- Tabbed grids, resizing, zoom, session snapshots, and activity summaries.
- Voice input and an opt-in local agent control API.
- Optional local background pane hosts with saved-session reattachment.
- Cross-platform native artifacts for Windows x64, Linux x64/arm64, and macOS
  arm64/x64.
- Stable-release gates tracked in [`V1_ACCEPTANCE.md`](V1_ACCEPTANCE.md).

## Current: Managed Agent Workspace

- Make commands and capabilities easier to discover.
- Improve per-pane activity, auth, and workload controls.
- Strengthen manager-driven implementation, review, test, and documentation
  loops.
- Improve first-run reliability and diagnostics across supported terminals.
- Turn early user feedback into small, testable workflow improvements.
- Grow a contributor path around scoped docs, tests, profiles, packaging, and
  TUI behavior.

## Stable V1

- Meet the release gates in [`V1_ACCEPTANCE.md`](V1_ACCEPTANCE.md) on every
  advertised platform.
- Keep CLI, config, session, and worktree behavior compatible and documented.
- Make release publication and installation dependable.
- Document default process ownership and optional background-terminal behavior
  clearly.

## Background Hosts And Future Multi-Client Attach

- Harden the shipped single-client background pane hosts and saved-session
  reattachment path.
- Safe multi-client attachment.
- A consolidated per-user session daemon and durable terminal state boundaries.
- Agent status classifiers and a stable plugin API.
- The proposed boundaries are in
  [`DAEMON_ARCHITECTURE.md`](DAEMON_ARCHITECTURE.md).

## Later Exploration

- Remote and SSH workspaces.
- A stable plugin or extension boundary.
- Recording and shareable workflow artifacts.
- Optional browser-based monitoring where it improves, rather than replaces,
  the native terminal workflow.

## Participate

Use [Discussions](https://github.com/jasonsuhari/gridbash/discussions) for
open-ended workflow and roadmap questions. Use
[`help wanted`](https://github.com/jasonsuhari/gridbash/issues?q=is%3Aissue%20state%3Aopen%20label%3A%22help%20wanted%22)
and
[`good first issue`](https://github.com/jasonsuhari/gridbash/issues?q=is%3Aissue%20state%3Aopen%20label%3A%22good%20first%20issue%22)
to find accepted work. Larger behavior changes should be discussed before
implementation.
