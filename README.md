# GridBash

[![CI](https://github.com/jasonsuhari/gridbash/actions/workflows/ci.yml/badge.svg)](https://github.com/jasonsuhari/gridbash/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/gridbash?label=npm)](https://www.npmjs.com/package/gridbash)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-0078D4.svg)](https://github.com/jasonsuhari/gridbash)

**The sexiest way to tokenmaxx.**

GridBash is a local workspace for running and coordinating CLI coding agents in
parallel. Launch, authenticate, isolate, monitor, and steer Codex, Claude, and
other agents side by side, each in a real PTY pane.

[Website](https://jasonsuhari.github.io/gridbash/) |
[npm](https://www.npmjs.com/package/gridbash) |
[Releases](https://github.com/jasonsuhari/gridbash/releases) |
[Full reference](docs/REFERENCE.md)

[![GridBash running six CLI coding agents in one terminal grid](https://raw.githubusercontent.com/jasonsuhari/gridbash/main/docs/assets/gridbash-openvid-demo-poster.png)](https://github.com/jasonsuhari/gridbash/blob/main/docs/assets/gridbash-openvid-demo.mp4)

## Quick start

Requires Node.js 18+. GridBash releases Windows x64, glibc-based Linux
x64/arm64, and macOS 13+ binaries for Apple Silicon and Intel.

```sh
npm install -g gridbash
gridbash
```

Or launch a six-pane Codex grid directly:

```sh
gridbash 2x3 --profile codex
```

The npm package installs only the native binary for your current platform.
The npm badge shows the version currently available from the registry. If it
temporarily trails the [latest GitHub release](https://github.com/jasonsuhari/gridbash/releases/latest),
use that release's matching native artifact until npm publication catches up.

## Why GridBash

- **Precise input routing.** Type into the focused pane, a selected set, or the
  entire grid.
- **Managed agent launch.** Choose the agent, auth profile, project, layout, and
  worktree policy before GridBash starts any panes.
- **Real terminals underneath.** Run up to 100 PTY-backed panes across tabbed
  grids, with raw shell grids still available as a secondary path.
- **Safer parallel work.** Give every pane an isolated repo-local git worktree.
- **Agent-first profiles.** Launch Codex, Claude, Gemini, Aider, OpenCode, Goose,
  Amp, Cursor, Copilot, shells, or custom commands.
- **Built-in workflow tools.** Resize grids, restore sessions, dictate prompts,
  inspect stable pane activity, optionally generate concise AI work summaries,
  ask BashBot to brief or coordinate the workspace, and let a manager route
  targeted follow-ups.
- **Optional background terminals.** Close the UI without stopping live panes,
  then reconnect to the same processes from a saved session.

## Common commands

| Command | Result |
| --- | --- |
| `gridbash` | Create a managed agent workspace interactively |
| `gridbash 2x3 --profile codex` | Launch a 2-by-3 Codex grid |
| `gridbash --count 12 --layout auto --profile claude` | Auto-arrange 12 Claude panes |
| `gridbash 2x3 --profile codex --worktrees` | Isolate every pane in a git worktree |
| `gridbash resume` | Choose a saved session to reopen |
| `gridbash resume --latest` | Reopen the latest saved session |
| `gridbash ctl list --json` | Discover opted-in running grids |
| `gridbash ctl panes --session ID` | Inspect numbered and stable pane identities |
| `gridbash --list-profiles` | Show detected profiles and resolved commands |
| `gridbash --help` | Show every CLI option |

`--worktrees` requires a git repository with at least one commit and no tracked
modifications. See the [reference](docs/REFERENCE.md#managed-git-worktrees) for
its folder, branch, and reuse behavior.

## Essential controls

GridBash shortcuts are modeless, so normal terminal keys continue to reach your
agents and shells.

| Input | Action |
| --- | --- |
| Drag mouse | Select and copy text inside one pane |
| Right-click pane | Add or remove the pane from the selected set |
| `Alt+k` | Search and run GridBash commands |
| `Alt` + arrow keys | Move focus between panes |
| `Alt+s` / `Alt+a` | Toggle the focused pane / select or clear all panes |
| `Alt+c` | Open or close the command line |
| `Alt+Shift+C` | Save bounded recent output from the target panes |
| `Alt+Shift+L` | Start or stop continuous target-pane logging |
| `Alt+d` | Chat with BashBot across all open grids and panes |
| `Alt+n` / `Alt+t` | Open a new tab / switch tabs |
| `Alt+p` | Open focused-pane activity |
| `Ctrl+Alt+p` | Inspect and stop localhost ports launched by agents |
| `Alt+Shift+A` | Manage auth profiles and assign one to the focused pane |
| `Alt+f` | Zoom or restore the focused pane |
| `Alt+b` | Search, select, and copy focused-pane scrollback |
| `Alt+Shift+b` / `Alt+Ctrl+b` | Background selected panes / open background agents |
| `Alt+g` / `Alt+u` | Start or stop the grid manager goal |
| `Alt+Shift+V` | Dictate one prompt without submitting it |
| `Alt+o` | Open settings |
| `Alt+h` or `F1` | Open the full in-app shortcut guide |
| `Alt+q` | Quit |

See the [full controls reference](docs/REFERENCE.md#controls) for resizing,
renaming, sleeping, restarting, scrolling, settings, and recovery actions.

To keep live terminals running after GridBash closes, open Settings with
`Alt+o` and enable **Keep terminals running**. GridBash returns control to the
launching shell when you quit; reconnect later with `gridbash resume --latest`
or select the session with `gridbash resume`.

If the terminal or GridBash process closes unexpectedly, the next plain
`gridbash` launch automatically recovers unfinished agent sessions. Saved panes
are grouped into tabs by working directory, each tab is named after that
directory, and `Alt+t` moves to the next tab. Explicit launch arguments still
start the workspace you requested, and older snapshots remain available through
`gridbash resume`.

## Profiles and configuration

A bare `gridbash` opens the agent-workspace setup. Detected agent profiles are
listed first; choose a compatible managed auth profile, project folder, grid
dimensions, and optional worktree isolation, then launch. Built-in shell
profiles remain available in the same screen as clearly labeled raw-terminal
options.

Managed auth applies to Claude or Codex processes GridBash launches. GridBash
does not install global shims, replace the normal `codex` or `claude` commands,
or intercept commands typed in an unmanaged shell.

Agent profiles are available on every platform: `codex`, `claude`, `gemini`,
`opencode`, `aider`, `amp`, `goose`, `copilot`, and `cursor`.
Profiles invoke CLIs already installed on your system; GridBash does not bundle
the agents themselves.

Terminal profiles are platform-specific:

```text
Windows:      git-bash pwsh powershell cmd
macOS/Linux:  zsh bash fish sh pwsh
```

Run `gridbash --list-profiles` to see what is available on your machine. Direct
launches resolve profiles in this order: `--profile`, `GRIDBASH_PROFILE`, the
invoking Windows shell, the configured default, then the platform default.

Start from [`config.example.toml`](config.example.toml) to define custom
profiles, UI settings, auth defaults, manager credentials, and workload policy.
The [configuration reference](docs/REFERENCE.md#configuration) covers file
locations and precedence.

Application shortcuts can also be remapped in `[keys]`, for example
`zoom-pane = "ctrl+shift+k"`. Unlisted actions keep their defaults, while F1
and `Alt+q` remain reliable help and quit fallbacks.

## Agent control API

Enable GridBash's local, opt-in control API for agents inside its panes:

```sh
gridbash --agent-api 2x3 --profile codex
```

Configure an agent MCP server to run `gridbash --mcp`. It can request a
lightweight grid snapshot, read bounded recent output from specific stable pane
IDs, show local images, send commands, capture or continuously log specific
panes, and update the GridBash status bar. Awareness is pull-based so agents can
request peer context only at coordination points; returned summaries and output
are explicitly untrusted context. The API is localhost-only,
token-authenticated, and off by default.

The same typed API is available to scripts through `gridbash ctl`. Discovery
metadata contains runtime IDs and localhost endpoints, never bearer tokens.
`ctl list` and `ctl panes` are read-only; send, capture, status, and focus
operations require `--token` or `GRIDBASH_CONTROL_TOKEN`. Child panes receive
the session ID and token automatically:

```sh
gridbash ctl list --json
gridbash ctl panes --session <id-or-prefix> --json
gridbash ctl send --session <id> --pane 2 "cargo test"
gridbash ctl focus --session <id> pane-4-gen-2
```

## Compatibility and current limits

- GridBash targets modern UTF-8, ANSI/xterm-compatible terminals and works over
  SSH or tmux when the remote session advertises a color-capable `TERM`.
- Use `--no-mouse` when a terminal or multiplexer does not forward mouse input.
  `TERM=dumb` and Linux kernel consoles are not supported.
- Background pane hosts are local and single-client. Closing GridBash can leave
  them running, but rebooting the machine or stopping a host loses the live PTY;
  saved history and launch metadata remain available for a fresh resume.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, validation, and pull request
guidance. Use `npm run install:local` for a local GridBash command; it installs a
packed copy instead of linking the command to a worktree.

Release maintainers should follow [docs/RELEASING.md](docs/RELEASING.md).

## Community

- Share your current setup in the
  [multi-agent workflow discussion](https://github.com/jasonsuhari/gridbash/discussions/256).
- Start with a
  [`good first issue`](https://github.com/jasonsuhari/gridbash/issues?q=is%3Aissue%20state%3Aopen%20label%3A%22good%20first%20issue%22)
  or an issue marked
  [`help wanted`](https://github.com/jasonsuhari/gridbash/issues?q=is%3Aissue%20state%3Aopen%20label%3A%22help%20wanted%22).
- Read [CONTRIBUTING.md](CONTRIBUTING.md) for setup, validation, and DCO
  guidance.
- Introduce yourself in the
  [new-contributor discussion](https://github.com/jasonsuhari/gridbash/discussions/257)
  if you want help choosing a task.
- See the [user and contributor growth playbook](docs/OUTREACH.md) if you want
  to help demonstrate GridBash, welcome testers, or recruit contributors.

## Project links

- [User reference](docs/REFERENCE.md)
- [Roadmap](docs/ROADMAP.md)
- [Devlogs](docs/devlogs/)
- [Outreach playbook](docs/OUTREACH.md)
- [Support](SUPPORT.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)

Created and maintained by [Jason Matthew Suhari](https://www.jasonsuhari.com).

GridBash is available under the [MIT License](LICENSE).
