# GridBash

Fast, beautiful terminal grids for running lots of CLI agents at once.

GridBash is a Windows-native Rust TUI multiplexer built for agent-heavy development: launch a grid of Codex, Claude, Gemini, Aider, OpenCode, Goose, Amp, Cursor, Copilot, Git Bash, PowerShell, or any custom command, then select panes and broadcast input only where you want it.

> V1 is intentionally single-process. Closing GridBash closes its child agents. Daemon detach/reattach is the next major frontier.

## Highlights

- Real PTY-backed panes through Windows ConPTY via `portable-pty`.
- Up to 100 panes in one terminal process.
- Configurable default terminal profile: Git Bash, PowerShell, cmd, agents, or custom.
- Native host-terminal text selection with no mouse-capture mode.
- Normal terminal keys pass through, including `Esc`, `Tab`, `Ctrl-a`, and `Ctrl-b`.
- Modeless Alt shortcuts for pane focus, selection, broadcast, resize, and quit.
- In-app terminal switching for focused or selected panes.
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

On first launch, if no default profile is configured, GridBash opens an animated setup screen and asks you to choose from the detected terminal profiles. The choice is saved to:

```text
%APPDATA%\GridBash\config.toml
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

GridBash does not capture the mouse, so normal drag selection and copy behavior stays owned by your host terminal. App controls use Alt shortcuts and do not require switching modes.

| Input | Action |
| --- | --- |
| Drag mouse | Select/copy terminal text in the host terminal |
| Alt+1 through Alt+9 / Alt+0 | Focus pane 1 through 10 |
| Alt+Left / Alt+Right | Focus previous / next pane |
| Alt+Up / Alt+Down | Focus pane above / below |
| Alt+s or Alt+Space | Toggle focused pane selection |
| Alt+a | Select all panes |
| Alt+c | Clear selection |
| Alt+b | Toggle selected broadcast mode |
| Alt+p | Show detected profile summary |
| Alt+t | Cycle target terminal profile forward |
| Alt+Shift+t | Cycle target terminal profile backward |
| Alt+Enter | Restart focused/selected panes with target profile |
| Alt+d | Save target profile as the default terminal |
| Alt+q | Quit |

When broadcast is on, typing goes to selected panes only. If nothing is selected, input goes to the focused pane.

Changing a pane's terminal restarts that pane, so shell state in that pane is discarded.

## Grid Resizing

Grid resizing is also modeless:

| Input | Action |
| --- | --- |
| Alt+Shift+Left | Narrow focused column |
| Alt+Shift+Right | Widen focused column |
| Alt+Shift+Up | Shorten focused row |
| Alt+Shift+Down | Heighten focused row |
| Alt+r | Reset equal grid |

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

## Legacy Launcher

The old Windows Terminal launcher is still useful for quick split-pane grids, but it cannot support true selected broadcast because Windows Terminal does not expose subset pane selection. The Rust app is the path forward.
