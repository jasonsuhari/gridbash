# GridBash

Fast, beautiful terminal grids for running lots of CLI agents at once.

GridBash is a Windows-native Rust TUI multiplexer built for agent-heavy development: launch a grid of Codex, Claude, Gemini, Aider, OpenCode, Goose, Amp, Cursor, Copilot, Git Bash, PowerShell, or any custom command, then select panes and broadcast input only where you want it.

> V1 is intentionally single-process. Closing GridBash closes its child agents. Daemon detach/reattach is the next major frontier.

## Highlights

- Real PTY-backed panes through Windows ConPTY via `portable-pty`.
- Up to 100 panes in one terminal process.
- Configurable default terminal profile: Git Bash, PowerShell, cmd, agents, or custom.
- Native host-terminal text selection with no mouse-capture mode.
- Normal terminal keys pass through to focused or broadcast panes.
- Modeless Alt shortcuts for pane focus, selection, broadcast, settings, and quit.
- Compact dark theme with focus, selection, activity, exit, and output-volume badges.
- Built-in launch profiles for common CLI coding agents.
- Guided orchestration composer for choosing folders, `vibe` auth profiles, and named setups.

## Install With npm

From this repo:

```powershell
npm install -g .
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

Open the guided composer:

```powershell
gridbash
```

The composer starts with the directory you launched `gridbash` from. It lets you choose folders, select logged-in `vibe` profiles, preview the pane-to-folder assignment, and optionally save the setup by name. GridBash uses `vibe run <profile> --` under the hood for isolated Claude/Codex auth.

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

Passing grid, count, profile, or cwd arguments bypasses the composer and uses the direct launch path.

## Composer Controls

| Input | Action |
| --- | --- |
| Up / Down | Move through choices |
| Enter | Continue, preview, or launch |
| Space | Toggle an agent profile |
| a | Add a folder on the folder screen; select all ready agents on the agent screen |
| d | Remove a selected folder |
| s | Save the previewed setup by name and launch |
| Esc | Go back |
| q | Quit from the composer |

## Controls

GridBash does not capture the mouse, so normal drag selection and copy behavior stays owned by your host terminal. App controls use Alt shortcuts and do not require switching modes.

| Input | Action |
| --- | --- |
| Drag mouse | Select/copy terminal text in the host terminal |
| Alt+Left / Alt+Right | Focus previous / next pane |
| Alt+Up / Alt+Down | Focus pane above / below |
| Alt+s or Alt+Space | Toggle focused pane selection |
| Alt+a | Select all panes |
| Alt+b | Toggle selected broadcast mode |
| Alt+o | Open sample settings |
| Alt+q | Quit |

When broadcast is on, typing goes to selected panes only. If nothing is selected, input goes to the focused pane.

The settings screen is currently a sample UI. Its switches, steppers, and choices can be changed, but they do not affect runtime behavior yet.

## Profiles

Built-in profile keys:

```text
git-bash powershell cmd codex claude gemini opencode aider amp goose copilot cursor
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

[setups.gridbash-swarm]
agents = ["claude-1", "claude-2", "codex-2"]

[[setups.gridbash-swarm.folders]]
name = "gridbash"
path = "C:\\Users\\Jason\\Documents\\GitHub\\gridbash"
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

## Legacy Launcher

The old Windows Terminal launcher is still useful for quick split-pane grids, but it cannot support true selected broadcast because Windows Terminal does not expose subset pane selection. The Rust app is the path forward.
