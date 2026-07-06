# GridBash

Fast, beautiful terminal grids for running lots of CLI agents at once.

GridBash is a Windows-native Rust TUI multiplexer built for agent-heavy development: launch a grid of Codex, Claude, Gemini, Aider, OpenCode, Goose, Amp, Cursor, Copilot, Git Bash, PowerShell, or any custom command, then select panes and broadcast input only where you want it.

> V1 is intentionally single-process. Closing GridBash closes its child agents. Daemon detach/reattach is the next major frontier.

## Highlights

- Real PTY-backed panes through Windows ConPTY via `portable-pty`.
- Up to 100 panes in one terminal process.
- Configurable default terminal profile: Git Bash, PowerShell, cmd, agents, or custom.
- Native host-terminal text selection by default.
- `Ctrl-m` toggles pane mouse controls.
- Ctrl-click toggles pane selection when pane mouse controls are enabled.
- Shift-click selects a range when pane mouse controls are enabled.
- `Ctrl-b` toggles selected broadcast mode.
- `Ctrl-g` opens spreadsheet-style grid resize mode.
- `Ctrl-a` selects every pane.
- `Esc` opens command mode.
- `Ctrl-q` exits.
- Mouse and keyboard navigation.
- Compact dark theme with focus, selection, activity, exit, and output-volume badges.
- Built-in launch profiles for common CLI coding agents.

## Install With npm

From this repo:

```powershell
npm install -g .
```

Then run it from anywhere:

```powershell
gridbash 2x3 --profile codex
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

Open the default 2x3 grid:

```powershell
gridbash
```

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

## Controls

By default, GridBash leaves mouse capture off so your host terminal can select and copy text normally. Press `Ctrl-m` to toggle pane mouse controls when you want click focus, pane selection, or grid divider dragging.

| Input | Action |
| --- | --- |
| Drag mouse | Select/copy terminal text in the host terminal |
| Ctrl-m | Toggle mouse text-selection / pane-control mode |
| Click pane | Focus pane in pane-control mode |
| Ctrl-click pane | Toggle pane selection in pane-control mode |
| Shift-click pane | Select range from focused pane in pane-control mode |
| Right-click pane | Toggle pane selection in pane-control mode |
| Drag left mouse | Add panes to selection in pane-control mode |
| Tab / Shift-Tab | Move focus |
| Ctrl-b | Toggle selected broadcast mode |
| Ctrl-g | Enter grid resize mode |
| Ctrl-a | Select all panes |
| Ctrl-q | Quit |
| Esc | Toggle command mode |

When broadcast is on, typing goes to selected panes only. If nothing is selected, input goes to the focused pane.

## Grid Resize Mode

Press `Ctrl-g` to enter GRID mode. Drag row or column dividers like a spreadsheet table, or use keyboard controls:

| Input | Action |
| --- | --- |
| Drag divider | Resize adjacent rows/columns |
| h / Left | Narrow focused column |
| l / Right | Widen focused column |
| k / Up | Shorten focused row |
| j / Down | Heighten focused row |
| = or 0 | Reset equal grid |
| Esc | Return to normal terminal input |

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
mouse_mode = "select"

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

Mouse mode values:

```text
select   host terminal owns mouse selection/copying
control  GridBash captures mouse for pane focus, pane selection, and grid resizing
```

## Design Goals

GridBash is inspired by agent-first multiplexers such as Mato and terminal workspaces such as Zellij, but V1 takes a different path: Windows-native, single binary, visual selection, selected broadcast, and a hard bias toward fast multi-agent grids.

## Legacy Launcher

The old Windows Terminal launcher is still useful for quick split-pane grids, but it cannot support true selected broadcast because Windows Terminal does not expose subset pane selection. The Rust app is the path forward.
