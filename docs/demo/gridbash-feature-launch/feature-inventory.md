# Feature Inventory and Coverage

The July 13 launch film covered only real terminal panes, selective broadcast,
managed worktrees, open source, and installation. This sequel groups the rest of
the current user-facing surface into readable chapters.

| Current capability | Source of truth | Film chapter |
| --- | --- | --- |
| Real PTY panes, one/some/all routing | README, Reference controls | 2 |
| Up to 100 panes and auto layout | Reference launch options | 2 |
| Managed worktrees | Reference managed worktrees | 2, 8 |
| Startup picker and tabbed grids | Reference startup/controls | 3 |
| Runtime resize with activity previews | Reference controls | 3 |
| Focused-pane zoom and pane swap | Reference controls | 3 |
| Wrapped focus navigation | Reference controls | 3 |
| Activity summaries and quiet/done states | Reference controls, current UI | 4 |
| Pane-local scrollback and contained copy | Reference controls | 4 |
| Previous panes list | Reference controls | 4 |
| Expanded command line | Reference controls | 5 |
| Push-to-talk dictation without submission | Reference voice mode | 5 |
| Rename, sleep/wake, restart, swap | Reference controls | 5 |
| Clipboard image-paste passthrough | v0.2 release notes | 5 |
| Nine built-in agent profiles and custom profiles | Reference profiles | 6 |
| Per-pane Claude/Codex auth profiles | Reference auth profiles | 6 |
| Auto-cycle and best-effort usage status | Reference auth profiles | 6 |
| Manager goals and targeted follow-ups | Reference grid manager | 7 |
| Idle TODO prompts and adaptive workload policy | Reference configuration | 7 |
| Local opt-in agent-control API | Reference agent control API | 7 |
| Bounded session resume | Reference sessions | 8 |
| Per-pane Codex SQLite lanes | Reference Codex SQLite isolation | 8 |
| Windows, Linux, and macOS native packages | package.json, v0.2 notes | 9 |
| Stable and nightly release channels | v0.2 notes, releasing docs | 9 receipt |

Release automation, issue labeling, review automation, internal reliability
fixes, and implementation-only performance changes remain written receipts or
devlog material rather than separate hero scenes because they are not primary
end-user workflows.
