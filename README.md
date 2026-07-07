# GridBash

[![npm version](https://img.shields.io/npm/v/gridbash?label=npm)](https://www.npmjs.com/package/gridbash)
[![GitHub release](https://img.shields.io/github/v/release/jasonsuhari/gridbash?label=github)](https://github.com/jasonsuhari/gridbash/releases)

Fast, beautiful terminal grids for running lots of CLI agents at once.

GridBash is a Windows-native Rust TUI multiplexer built for agent-heavy development: launch a grid of Codex, Claude, Gemini, Aider, OpenCode, Goose, Amp, Cursor, Copilot, Git Bash, PowerShell, or any custom command, then select panes and broadcast input only where you want it.

> V1 is intentionally single-process. Closing GridBash closes its child agents. Daemon detach/reattach is the next major frontier.

## Release Status & Devlogs

- Latest npm version is shown by the npm badge above and on the npm package page.
- Latest GitHub release is shown by the GitHub release badge above and on the GitHub Releases page.
- Devlogs live in `docs/devlogs/`.
- Versioned release notes live in `docs/releases/` and are used for GitHub release notes.
- npm packages include `docs/devlogs/` and `docs/releases/` so published package contents carry the logs too.

## Highlights

- Real PTY-backed panes through Windows ConPTY via `portable-pty`.
- Up to 100 panes in one terminal process.
- Configurable default terminal profile: Git Bash, PowerShell, cmd, agents, or custom.
- Native host-terminal text selection with no mouse-capture mode.
- Normal terminal keys pass through to focused or broadcast panes.
- Modeless Alt shortcuts for pane focus, selection, broadcast, settings, and quit.
- Compact dark theme with focus, selection, activity, exit, and output-volume badges.
- Quiet-output indicators call out panes that produced output and then stopped with a palette-controlled border and icon.
- Built-in launch profiles for common CLI coding agents.
- Startup dimension picker with a live grid preview.

## Install With npm

From this repo:

```powershell
npm run install:local
```

Then run it from anywhere:

```powershell
gridbash
```

Build a publishable npm tarball:

```powershell
npm pack
```

The package ships a Node command shim that launches the bundled Windows x64 `gridbash.exe`.

Release automation and devlog workflow are documented in `docs/RELEASING.md`.

Use `npm run install:local` for local development installs. It installs from a packed tarball so the global `gridbash` command points at a stable package copy, not whichever `.worktrees/` checkout last ran `npm install -g .`.

## PR Workflow

Pull requests can be merged directly after they have been reviewed. Before merging, check the diff, confirm the intent is clear, and make sure the relevant validation has passed.

## Install From Source

Install Rust first:

```powershell
winget install --id Rustlang.Rustup -e
```

Build GridBash:

```powershell
git clone https://github.com/jasonsuhari/gridbash
cd gridbash
cargo build --release
```

The executable will be:

```text
target\release\gridbash.exe
```

## Use

Open the startup grid picker:

```powershell
gridbash
```

On first launch, if no default profile is configured, GridBash opens an animated setup screen and asks you to choose from the detected terminal profiles. The choice is saved to:

```text
%APPDATA%\GridBash\config.toml
```

The startup picker asks for rows and columns, updates the preview grid as you change them, and launches every pane in the directory where you started `gridbash`.

Set the default terminal profile:

```powershell
gridbash --set-default powershell
```

Open a specific grid:

```powershell
gridbash 2x3 --profile git-bash
```

Open 12 panes and auto-arrange them:

```powershell
gridbash --count 12 --layout auto --profile claude
```

List detected profiles:

```powershell
gridbash --list-profiles
```

Start in a repo:

```powershell
gridbash 3x4 --profile codex --cwd C:\Users\Jason\Documents\GitHub\fluent
```

Passing grid, count, profile, or cwd arguments bypasses the startup picker and uses the direct launch path.

## Startup Picker Controls

| Input | Action |
| --- | --- |
| Left / Right | Switch between rows and columns |
| Up / Down | Increase or decrease the active dimension |
| r / c | Select rows or columns |
| 1-9 / 0 | Set the active dimension directly, with 0 meaning 10 |
| Enter | Launch the grid |
| Esc / q | Quit |

## Controls

GridBash does not capture the mouse, so normal drag selection and copy behavior stays owned by your host terminal. App controls use Alt shortcuts and do not require switching modes.

| Input | Action |
| --- | --- |
| Drag mouse | Select/copy terminal text in the host terminal |
| Alt+Left / Alt+Right | Focus previous / next pane |
| Alt+Up / Alt+Down | Focus pane above / below |
| Alt+s | Toggle focused pane selection |
| Alt+a | Select all panes, or clear selection when all panes are selected |
| Alt+b | Toggle selected broadcast mode |
| Alt+o | Open settings |
| Alt+q | Quit |

When broadcast is on, typing goes to selected panes only. If nothing is selected, input goes to the focused pane.

Pane titles show `active` while fresh output is arriving and `exited` after the child process ends. After roughly three seconds without output, a pane gets a colored border and a small `●` title icon. The icon is an output signal, not a guarantee that an agent has completed its task.

The settings screen includes live color controls for GridBash's accent, focus, selected, active, quiet, and exited grid roles. Changes apply immediately for the current run.

## Profiles

Built-in profile keys:

```text
git-bash pwsh powershell cmd codex claude gemini opencode aider amp goose copilot cursor
```

GridBash resolves Windows `.exe` and `.cmd` shims before extensionless npm shims, so common Node-based CLIs launch correctly.

Optional config file:

```text
%APPDATA%\GridBash\config.toml
```

Example:

```toml
[defaults]
profile = "powershell"

[profiles.review]
command = "codex"
args = ["--model", "gpt-5.5"]
title = "Codex Review"
```

Then run:

```powershell
gridbash 2x4 --profile review
```

Default profile resolution order:

```text
--profile > GRIDBASH_PROFILE > [defaults].profile > git-bash
```

## Design Goals

GridBash is inspired by agent-first multiplexers such as Mato and terminal workspaces such as Zellij, but V1 takes a different path: Windows-native, single binary, visual selection, selected broadcast, and a hard bias toward fast multi-agent grids.

## Community

- Read `CONTRIBUTING.md` before opening a pull request.
- See `docs/ROADMAP.md` for the release roadmap.
- Use GitHub Issues for actionable bugs, tasks, and feature requests.
- Use GitHub Discussions for questions, ideas, and longer design conversation.
- Follow `SECURITY.md` for private vulnerability reports.

## Legacy Launcher

The old Windows Terminal launcher is still useful for quick split-pane grids, but it cannot support true selected broadcast because Windows Terminal does not expose subset pane selection. The Rust app is the path forward.
