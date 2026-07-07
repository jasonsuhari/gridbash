# GridBash

[![npm version](https://img.shields.io/npm/v/gridbash?label=npm)](https://www.npmjs.com/package/gridbash)
[![GitHub release](https://img.shields.io/github/v/release/jasonsuhari/gridbash?label=github)](https://github.com/jasonsuhari/gridbash/releases)

Fast, beautiful terminal grids for running lots of CLI agents at once.

GridBash is a Windows-native Rust TUI multiplexer built for agent-heavy development: launch a grid of Codex, Claude, Gemini, Aider, OpenCode, Goose, Amp, Cursor, Copilot, Git Bash, PowerShell, or any custom command, then select panes and send input only where you want it.

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
- Pane-contained drag selection that copies selected terminal text without crossing into sibling panes.
- Normal terminal keys pass through to the focused pane, or to selected panes when multiple panes are selected.
- Modeless Alt shortcuts for pane focus, selection, rename, settings, and quit.
- Compact dark theme with focus, selection, activity, exit, and output-volume badges.
- Claude, Codex, and other agent panes show a compact conversation summary in the footer line.
- Built-in launch profiles for common CLI coding agents.
- Startup dimension picker with a live grid preview.
- Optional managed git worktrees so every pane can work in an isolated checkout.

## Demo

Watch the OpenVid-style demo: [`docs/assets/gridbash-openvid-demo.mp4`](docs/assets/gridbash-openvid-demo.mp4).

The source scene and OpenVid recreation recipe are in [`docs/demo/openvid-gridbash-demo.md`](docs/demo/openvid-gridbash-demo.md).

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

Launch every pane in a separate repo-local git worktree:

```powershell
gridbash 2x3 --profile codex --worktrees
```

With `--worktrees`, GridBash creates or reuses `.worktrees/gridbash-<base>-NN` folders and `gridbash/<base>-pane-NN` branches. Panes keep the same relative folder as the directory where you launched GridBash, so starting from `repo\app` opens each terminal in the matching `app` folder inside its managed worktree. GridBash refuses this mode outside a git repo or when tracked changes are present in the base checkout.

You can also run `gridbash --worktrees` and choose the grid dimensions in the startup picker.

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

GridBash captures drag selection so selected text stays inside the pane where the drag started. Releasing the drag sends the selected terminal text to the host clipboard through the standard OSC 52 terminal clipboard sequence. App controls use Alt shortcuts and do not require switching modes.

| Input | Action |
| --- | --- |
| Drag mouse | Select/copy terminal text within the source pane |
| Alt+Left / Alt+Right | Focus previous / next pane |
| Alt+Up / Alt+Down | Focus pane above / below |
| Alt+Shift+Up / Alt+Shift+Down | Remove / add a row when safe |
| Alt+Shift+Left / Alt+Shift+Right | Remove / add a column when safe |
| Alt+s | Toggle focused pane selection |
| Alt+a | Select all panes, or clear selection when all panes are selected |
| Alt+r | Rename the focused pane |
| Alt+o | Open sample settings |
| Alt+q | Quit |

Typing goes to selected panes whenever multiple panes are selected. With zero or one pane selected, input goes to the focused pane.

Renamed pane headers replace the numeric prefix for the current session. Saving a blank name restores the default number.

The settings screen is currently a sample UI. Its switches, steppers, and choices can be changed, but they do not affect runtime behavior yet.

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

GridBash is inspired by agent-first multiplexers such as Mato and terminal workspaces such as Zellij, but V1 takes a different path: Windows-native, single binary, visual selection, scoped multi-pane input, and a hard bias toward fast multi-agent grids.

## Community

- Read `CONTRIBUTING.md` before opening a pull request.
- See `docs/ROADMAP.md` for the release roadmap.
- Use GitHub Issues for actionable bugs, tasks, and feature requests.
- Use GitHub Discussions for questions, ideas, and longer design conversation.
- Follow `SECURITY.md` for private vulnerability reports.

## Legacy Launcher

The old Windows Terminal launcher is still useful for quick split-pane grids, but it cannot support true subset pane input because Windows Terminal does not expose subset pane selection. The Rust app is the path forward.
