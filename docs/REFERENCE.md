# GridBash Reference

This guide covers launch options, sessions, managed worktrees, controls, profiles,
configuration, and platform-specific behavior. See the [project
README](../README.md) for installation and a shorter introduction.

## Launch grids

Run `gridbash` with no arguments to open the row-and-column picker. The picker launches panes in the directory where GridBash was started.

Common direct launches:

```powershell
# Fixed grid
gridbash 2x3 --profile codex

# Auto-arrange a pane count
gridbash --count 12 --layout auto --profile claude

# Start elsewhere
gridbash 3x4 --profile codex --cwd C:\path\to\repo

# Use a custom config file
gridbash 2x3 --config C:\path\to\gridbash.toml
```

The main launch options are:

| Option | Behavior |
| --- | --- |
| `ROWSxCOLS` | Set an explicit grid, such as `2x3`. |
| `--count N` | Launch `N` panes, up to 100. |
| `--layout auto` | Derive the grid dimensions from the pane count. |
| `--profile NAME` | Use one built-in or custom profile for all panes. |
| `--cwd PATH` | Set the panes' starting directory. |
| `--worktrees` | Give each pane a managed git worktree. |
| `--worktree-prefix NAME` | Change the managed folder and branch prefix from `gridbash`. |
| `--config PATH` | Load and save an alternate TOML configuration file. |
| `--no-mouse` | Leave mouse handling to the host terminal. |
| `--agent-api` | Enable the local agent control API. |
| `--agent-api-port PORT` | Choose the port for the enabled API; `0` selects a free port. A nonzero value also enables the API. |

Grid, count, profile, cwd, or auto-layout arguments use the direct launch path and bypass the startup picker. `gridbash --worktrees` by itself still opens the picker.

On first launch without a CLI profile, environment override, detected invoking shell, or configured default, GridBash asks you to choose from the terminal profiles it can find. The choice is saved to `%APPDATA%\GridBash\config\config.toml` on Windows. Change it later with:

```powershell
gridbash --set-default powershell
```

### Startup picker

| Input | Action |
| --- | --- |
| Left / Right | Switch between rows and columns. |
| Up / Down | Increase or decrease the active dimension. |
| `r` / `c` | Select rows or columns. |
| `1`-`9` / `0` | Set the active dimension; `0` means 10. |
| Enter | Launch the grid. |
| Esc / `q` | Quit. |

## Sessions and resume

GridBash writes bounded session snapshots to local app data when grids launch and exit. Resume interactively, resume the latest snapshot, list snapshots, or select a session by its full ID or unique prefix:

```powershell
gridbash resume
gridbash resume --latest
gridbash resume --list
gridbash resume <session-id>
```

A snapshot restores grid dimensions, pane profiles, working directories,
worktree names, auth assignments, and a pane-local view of recent submitted
commands and output. Resume starts new child terminals. It does not reattach old
processes or replay commands into a shell.

GridBash is currently single-process: closing it closes its child agents. Session resume is recovery context, not detach and reattach.

## Managed git worktrees

Use `--worktrees` to isolate every pane in a repo-local checkout:

```powershell
gridbash 2x3 --profile codex --worktrees
gridbash 2x3 --profile codex --worktrees --worktree-prefix review
```

With the default prefix, GridBash creates or reuses:

```text
.worktrees/gridbash-<base>-NN
gridbash/<base>-pane-NN
```

The first pattern is the folder and the second is its branch. A custom prefix replaces `gridbash` in both.

GridBash preserves the launch directory relative to the repository root. Starting from `repo\app`, for example, opens each pane in the matching `app` directory inside its worktree.

Managed mode requires a git repository with at least one commit and refuses to
start when the base checkout has tracked changes. Untracked files do not block
launch. Existing branches and worktrees are reused only when they match the
expected repository and branch.

## Agent control API

The opt-in agent API lets tools running in a pane control the current GridBash session:

```powershell
gridbash --agent-api 2x3 --profile codex
```

Child panes receive `GRIDBASH_CONTROL_ADDR`, `GRIDBASH_CONTROL_TOKEN`, and the 1-based `GRIDBASH_PANE_INDEX`. Configure an agent's MCP client to run this stdio server from a pane:

```powershell
gridbash --mcp
```

The MCP server exposes:

| Tool | Behavior |
| --- | --- |
| `gridbash_show_image` | Display a local PNG, JPEG, GIF, or WebP file in an overlay. |
| `gridbash_send_command` | Send text to one or more 1-based pane numbers; submitting with Enter is optional. |
| `gridbash_set_status` | Replace the current session's status-bar message. |

The API binds only to localhost, authenticates with a per-session token, and is disabled by default.

## Controls

GridBash is modeless: ordinary terminal input continues to the active target, while application commands use Alt shortcuts.

| Input | Action |
| --- | --- |
| Drag mouse | Select terminal text inside the pane where the drag began and copy it on release. |
| Right-click pane | Add or remove that pane from the selected set. |
| Mouse wheel | Scroll only the pane under the pointer; selected panes use GridBash scrollback. |
| Alt+Left / Alt+Right | Focus the previous or next pane in the row, wrapping at the edge. |
| Alt+Up / Alt+Down | Focus the pane above or below, wrapping at the edge. |
| Alt+l | Resize the current grid. |
| Alt+x | Swap the two selected panes. |
| Alt+n | Open the startup picker and launch a new tab. |
| Alt+t | Switch to the next tab. |
| Alt+s | Toggle selection of the focused pane. |
| Alt+a | Select all panes, or clear the set when all are selected. |
| Alt+c | Open or close the expanded command line. |
| Alt+f | Zoom the focused pane to the full grid area, or restore the grid. |
| Alt+Shift+V | Dictate one utterance, or cancel active listening. |
| Alt+h / F1 | Open or close help. |
| Alt+p | Open the focused-pane activity summary. |
| Alt+Shift+P | Open the previous-panes list. |
| Alt+b | Move selected panes, or the focused pane, into the background and launch fresh replacements. |
| Alt+Shift+B | Open the session-wide background-agents list. |
| Alt+r | Rename the focused pane. |
| Alt+Shift+R | Rename the current tab. |
| Alt+Shift+T | Restart the exited focused pane, or all exited selected panes. |
| Alt+z | Sleep the focused pane, or all selected panes. |
| Hover sleeping pane | Wake it and reveal its terminal. |
| Alt+g | Create or edit the current grid's manager goal. |
| Alt+u | Stop the current grid's manager goal. |
| Alt+o | Open settings. |
| Alt+q | Quit. |

Drag selection is contained to its source pane and copies through the standard OSC 52 clipboard sequence. Use `--no-mouse` if the host terminal, serial link, or multiplexer cannot forward mouse reporting.

When multiple panes are selected, typing is broadcast to them. With zero or one selected pane, input goes only to the focused pane. The Alt+c command line captures its output and runs Enter-submitted commands in the cwd shown in its prompt.

Pane Activity provides auth, rename, refresh, sleep/wake, and manager-goal controls. Navigate with Up/Down and activate with Enter or Space. Direct keys inside the view are `n` to rename, `r` to refresh, `z` to sleep or wake, `g` to edit the grid goal, and `u` to stop it. Close it with Esc, `q`, or Alt+p; Alt+o switches to overall settings.

Alt+B backgrounds every explicitly selected pane, or the focused pane when the selected set is empty. GridBash first launches fresh panes with the same profile, command, auth, folder, and worktree; only after all replacements succeed are the original PTYs moved into the session-wide pool. Custom pane names follow the original jobs, while fresh replacements return to numbered labels.

Alt+Shift+B opens Background Agents. Rows show whether each job is working, quiet, exited, or offline, together with its agent, source tab, folder/worktree, and latest activity. Use Up/Down to choose a row and Enter or Space to swap it into the focused cell; the displaced visible pane goes into the pool instead of being terminated. `R` explicitly restarts an exited or offline row. Delete removes exited/offline rows immediately and requires a second press before stopping a live process.

If the focused pane exits, Enter, `r`, or `t` restarts it, while `z` sleeps it. Alt+Shift+T performs the same restart directly for exited target panes.

The resize picker starts from the current dimensions and shows each existing pane's latest activity summary when one is available. Shrinking a grid deactivates live panes outside the retained upper-left rectangle; changing `3x3` to `3x2`, for example, removes the rightmost column.

A pane's top border shows its latest activity summary. A configured manager goal replaces that summary across the grid until removed. A quiet marker appears after roughly three seconds without output; it indicates output followed by inactivity, not completion or process exit. Saving a blank pane name restores its default number.

## Voice mode

Press Alt+Shift+V to listen for one utterance, for up to 15 seconds. The transcript goes to the command line or to the panes targeted when listening began. GridBash never presses Enter for dictated text, so it can be reviewed before submission. Press the shortcut again to cancel.

### Windows

Windows dictation uses Microsoft's online speech service. Enable **Online speech recognition** under **Privacy & security > Speech**, allow desktop applications to use the microphone, and install the speech language pack for the desired dictation language. GridBash reports the platform error when a requirement is missing.

### macOS

GridBash asks for Speech Recognition and Microphone access on first use. It prefers on-device recognition and uses Apple's authorized speech service when the current locale does not support local recognition.

### Linux

Linux voice mode uses offline Whisper. The first shortcut explains that a 57 MiB model is required; press it again to approve the one-time download. GridBash checksum-verifies the model and stores it in the local XDG data directory, and audio stays on the machine.

Set `GRIDBASH_VOICE_MODEL` to use another local Whisper model or `GRIDBASH_SPEECH_HELPER` to replace the packaged helper. Capture uses ALSA and may need explicit device access in containers or remote sessions.

## Terminal compatibility

GridBash targets modern UTF-8, ANSI/xterm-compatible terminals, including Windows Terminal, Apple Terminal, iTerm2, GNOME Terminal, Konsole, Kitty, WezTerm, and Alacritty.

SSH and tmux work when the remote session advertises a color-capable `TERM`. `TERM=dumb` and Linux kernel consoles are unsupported. In Apple Terminal and iTerm2, configure Option as Meta/Alt so GridBash receives its shortcuts.

## Launch profiles

Built-in terminal profiles are platform-specific:

| Platform | Profile keys |
| --- | --- |
| Windows | `git-bash`, `pwsh`, `powershell`, `cmd` |
| macOS | `zsh`, `bash`, `fish`, `sh`, `pwsh` |
| Linux | `zsh`, `bash`, `fish`, `sh`, `pwsh` |

Agent profiles available on every platform are `codex`, `claude`, `gemini`, `opencode`, `aider`, `amp`, `goose`, `copilot`, and `cursor`.

Inspect every built-in and custom profile with:

```powershell
gridbash --list-profiles
```

The diagnostic table identifies the selected default, source, availability, resolved executable, or missing-command reason. It never prints profile environment values, auth tokens, or manager credentials. On Windows, GridBash resolves `.exe` and `.cmd` shims before extensionless npm shims.

Profile selection uses this precedence:

1. `--profile`
2. `GRIDBASH_PROFILE`
3. The invoking Windows shell detected by the npm launcher
4. `[defaults].profile`
5. The platform default

The platform default is Git Bash on Windows, zsh on macOS, and bash on other Unix systems. On Windows, the npm launcher can inherit PowerShell, PowerShell 7 (`pwsh`), cmd, or Git Bash from the shell that invoked `gridbash`.

Define custom profiles under `[profiles.<name>]`:

```toml
[profiles.review]
command = "codex"
args = ["--model", "gpt-5.5"]
title = "Codex Review"
agent_kind = "codex"
```

Then launch it by key:

```powershell
gridbash 2x4 --profile review
```

`agent_kind` is optional. Set it to `claude` or `codex` when the profile should participate in that agent's auth handling.

## Configuration

The default configuration file is platform-specific:

| Platform | Path |
| --- | --- |
| Windows | `%APPDATA%\GridBash\config\config.toml` |
| macOS | `$HOME/Library/Application Support/GridBash/config.toml` |
| Linux | `${XDG_CONFIG_HOME:-$HOME/.config}/gridbash/config.toml` |

Use `--config PATH` to load and save another file. A representative configuration is:

```toml
[defaults]
profile = "powershell"
pane_priority = "below-normal" # or "normal"
pane_workload = "adaptive"     # or "unrestricted"

[ui]
compact_titles = false
activity_badges = true
confirm_quit = false
scrollback_rows = 10000
refresh_ms = 16

[manager]
endpoint = "https://api.openai.com/v1/chat/completions"
model = "gpt-4o-mini"
api_key = "sk-..."

[todos]
idle_seconds = 90
prompts = [
  "Review the latest changes and summarize anything risky.",
  "Run the fastest relevant validation and report failures.",
]

[auth]
home = "C:\\Users\\you\\.gridbash-auth"
auto_cycle = false
usage_status = true

[auth.defaults]
claude = "claude-1"
codex = "codex-2"
```

Settings persist compact titles, activity badges, quit confirmation, new-pane scrollback, refresh delay, todo prompts, auth and workload policy, and the interface palette. Supported runtime changes apply immediately.

### Grid manager

The manager uses the OpenAI-compatible chat-completions endpoint, model, and API key under `[manager]`. These values can also be edited in Settings > Manager. The UI masks the API key, but the key is stored in the local TOML file.

Alt+G creates a goal for the current grid. Each review sends pane role and folder metadata plus bounded recent output from every awake pane to the configured API. Sleeping and exited panes are omitted from reviews and are never command targets. Reviews label output by pane, and validated follow-ups remain bound to their intended PTYs if panes are reordered.

### Pane priority and workload

On Windows, the GridBash interface remains at normal process priority while pane processes default to `below-normal`; child workloads normally inherit the pane priority. Set `[defaults].pane_priority = "normal"` to opt out.

The default `adaptive` workload policy gives focused and selected panes more CPU time than hidden or sleeping panes when Windows is contested, while every pane keeps running. Set `[defaults].pane_workload = "unrestricted"` or change Workload policy in Performance settings to disable adaptive sharing.

## Auth profiles

GridBash can isolate Claude and Codex accounts in named directories. The auth home is resolved in this order:

```text
GRIDBASH_AUTH_HOME > [auth].home > CLAUDE_PROFILES_HOME (legacy) > ~/.gridbash-auth
```

Claude panes receive `CLAUDE_CONFIG_DIR=<profile-dir>` and Codex panes receive `CODEX_HOME=<profile-dir>`.

The old default was `~/.claude-profiles`; profiles are not moved automatically. Move them to `~/.gridbash-auth`, point `[auth].home` at the old location, or keep the legacy `CLAUDE_PROFILES_HOME` override.

Assignment is manual by default: a new pane uses the configured default for its agent kind, while an explicit per-pane selection is retained. With `auto_cycle = true`, new compatible panes are assigned round-robin across ready profiles of the same kind. Changing the policy does not restart panes already running.

### Auth settings controls

| Input | Action |
| --- | --- |
| Tab | Switch Settings tabs. |
| Up / Down | Move through profiles. |
| `d` | Make the selected profile the GridBash-wide default for its kind. |
| `c` | Toggle manual assignment and auto-cycle for new panes. |
| `n` | Create a profile directory. |
| `l` | Open the selected profile's login command. |
| `r` | Refresh local account and usage status. |
| Esc / `q` | Close settings. |

For a Claude or Codex pane, open Pane Activity with Alt+p, select the auth control with Up/Down, choose a compatible profile with Left/Right, and press Enter. Applying a different account restarts only that pane. Press `r` in Pane Activity to refresh its snapshot.

Usage reporting is best-effort. GridBash reads local auth metadata, masks account email addresses, and makes short-timeout requests with `curl.exe` on Windows or `curl` on macOS only when the Auth view is refreshed. Disable it with `usage_status = false`.

### Codex SQLite isolation

GridBash leases each pane a unique, persistent `CODEX_SQLITE_HOME`, including terminal-profile panes where Codex is started manually. Lanes are separated by `CODEX_HOME`, protected by cross-process file locks, and reused only after their previous pane releases the lease. This prevents concurrent Codex processes from contending for the same SQLite databases while auth, configuration, skills, and rollout files remain shared within `CODEX_HOME`.

The first use of a new lane can be slower while Codex indexes existing rollouts. SQLite-only state, including goals, memories, and thread relationships, remains local to that lane.

A non-empty `CODEX_SQLITE_HOME` inherited by GridBash opts out of automatic isolation and is preserved. Codex's `sqlite_home` configuration keeps its normal precedence. Do not point concurrent panes at the same override, or SQLite lock contention can return.
