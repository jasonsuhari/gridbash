# Building a Windows PTY Grid for Coding Agents in Rust

> Publication draft. Adapt the introduction and examples to the destination
> before publishing.

The first time I ran several coding agents in parallel, the hard part was not
starting them. The hard part was keeping track of them.

One terminal was implementing. Another was reviewing. A third was running tests.
A fourth had stopped to ask a question I did not notice. Adding more windows
made every session harder to scan, while broadcasting input through ordinary
terminal tooling made it too easy to send a prompt somewhere it did not belong.

I built [GridBash](https://github.com/jasonsuhari/gridbash) to explore a narrow
idea: what if the terminal itself understood the workflow of running several
independent CLI coding agents?

GridBash is a Rust TUI that puts real PTY-backed sessions into one terminal
grid. It does not replace Codex, Claude, Gemini, Aider, or other agents. It
launches their normal CLI interfaces, keeps them visible, and gives the user an
explicit way to choose which panes receive input.

This article covers the engineering boundaries that mattered most.

## Preserve the native CLI

An early architectural choice was whether to integrate with agent APIs or keep
the agents as terminal processes.

The API route offers deep control, but it also turns the multiplexer into an
agent framework. Every provider has different authentication, streaming,
tool-use, approval, and conversation semantics. Supporting a tool would mean
reimplementing part of its interface and keeping that adapter current.

A PTY boundary is smaller and more durable. If a command works in a terminal,
GridBash can host it. Authentication remains the tool's responsibility. The
agent keeps its native colors, keybindings, approval prompts, and update cycle.
Custom commands work through the same path as built-in profiles.

This is also an honest description of the orchestration involved. GridBash
orchestrates terminals and input routing; it does not claim that unrelated
agents suddenly share memory or a common protocol.

## A pane is a terminal, not a text box

Rendering command output is the easy-looking part of a terminal multiplexer.
The difficult part is preserving terminal behavior.

Interactive programs expect a pseudo-terminal. They move the cursor, redraw
regions, switch screen buffers, negotiate size, emit color and style sequences,
and react to control keys. Treating their output as lines of text breaks as soon
as a program behaves like a real TUI.

GridBash uses `portable-pty` to create PTY-backed child sessions and Ratatui for
the outer interface. On Windows, those sessions run through ConPTY. Each pane
owns its terminal state, process lifecycle, title, scrollback, and rendered
region. Resizing a pane must propagate a new terminal size to the child rather
than merely changing how many cells the parent draws.

That boundary keeps the design understandable:

- The child process believes it owns a normal terminal.
- The terminal parser owns the child's screen state.
- The GridBash UI decides where that screen appears.
- The input router decides which PTYs receive a user's bytes.

## Input routing should be visible

Broadcast input is powerful and dangerous for the same reason: one action can
affect many processes.

GridBash separates focus from selection. Focus answers “which pane am I looking
at?” Selection answers “which panes should receive this input?” A user can send
input to one pane, a chosen set, or the whole grid, but the destination must be
visible before the bytes are written.

This is especially important for coding agents. Two sessions may look similar
while operating on different tasks, repositories, or approval prompts. A
command intended for a test runner should not quietly become a response to an
agent asking permission to modify files.

The outer TUI therefore needs modeless controls that coexist with normal
terminal input. Most keystrokes pass through untouched. A small set of Alt
shortcuts changes focus, toggles selection, or invokes GridBash behavior. The
goal is to make routing explicit without turning every interaction into a mode
switch.

## Mouse selection must stop at pane boundaries

Terminal text selection looks like a cosmetic detail until multiple terminal
surfaces share one viewport.

The parent application receives mouse coordinates for the entire grid. The
selected child terminal only understands coordinates inside its own cell
region. Dragging across a border cannot be allowed to select text from a sibling
pane or change the input destination halfway through the gesture.

The useful invariant is simple: a selection belongs to the pane where it began.
Coordinates are translated into that pane's local cell space and clamped to its
content area. The gesture ends there even if the pointer later crosses another
pane.

This kind of invariant is worth expressing in code instead of relying on the UI
to “usually” behave. Agent-heavy workflows make accidental cross-pane actions
more expensive than they are in a grid of ordinary shells.

## Parallel agents need filesystem isolation

Running six agents in parallel is not useful if they all edit the same working
tree.

GridBash can create a repo-local git worktree for each pane. Every agent receives
an isolated checkout while the user retains one visual surface for the whole
operation. An implementation pane can change code while a review pane examines
the base branch and a test pane validates another candidate.

Git worktrees are a pragmatic boundary because they reuse a tool developers
already understand. There is no custom snapshot format or hidden copy of the
repository. Branches, diffs, and cleanup remain visible through normal git
commands.

They also expose the real coordination problem: isolation prevents file
collisions, but it does not decide which result should win. Review, integration,
and merge decisions still belong to the developer. The terminal grid makes the
work observable; it does not pretend that concurrency removes the need for
judgment.

## Many panes change the redraw budget

A single terminal can redraw aggressively without feeling expensive. A grid of
live agents produces output in bursts across several panes, often while the user
is typing into one of them.

The outer application has to distinguish state changes that require a redraw
from background activity that can be coalesced. It also needs to prevent noisy
panes from making the focused pane feel sluggish. Sleeping or quiet panes should
not consume the same visual attention as a pane waiting for input.

The practical lesson is to measure the hot path around output ingestion,
terminal parsing, layout, and paint as one system. Optimizing only the Rust UI
loop will not help if every PTY event still triggers unnecessary work elsewhere.

GridBash exposes pane state such as focus, selection, sleep, exit, and quiet
output directly in the grid. Those cues are not decoration; they let the user
scan many concurrent processes without reading every line.

## What V1 deliberately does not solve

GridBash V1 is single-process. Closing GridBash closes its child agents.

That limitation keeps process ownership and cleanup straightforward, but it
means the application is not yet a durable session daemon. Proper detach and
reattach support requires a different boundary: a long-lived process must own
PTYs, buffer state, authenticate clients, recover terminal dimensions, and
reconnect the UI without losing process semantics.

Calling that out matters. A terminal multiplexer is trusted with long-running
work, so persistence should be designed as an invariant rather than added as a
background-process trick.

## Try it

The currently published package supports Windows x64:

```powershell
npm install -g gridbash
gridbash 2x3 --profile codex --worktrees
```

GridBash is MIT licensed. The source, demo, issues, and roadmap are available at
[github.com/jasonsuhari/gridbash](https://github.com/jasonsuhari/gridbash).

The feedback I am most interested in is concrete workflow friction: how many
agent sessions do you actually run, how do you separate their repositories, and
where does your current terminal setup become unsafe or difficult to scan?
