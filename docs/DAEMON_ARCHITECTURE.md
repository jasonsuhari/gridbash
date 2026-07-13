# Daemon Detach and Reattach Architecture

## Status and goal

This is a design proposal, not the v1 runtime contract. GridBash v1 intentionally
owns PTYs inside the foreground TUI process. The next major architecture should
let a user detach the UI while pane processes continue, then attach one or more
clients without changing ordinary single-machine workflows.

The first daemon release should preserve local PTYs and local trust boundaries.
Remote access, web clients, plugins, and multi-host orchestration should build on
the protocol later instead of expanding the first implementation.

## Process boundary

`gridbashd` is the long-lived owner of sessions, PTY masters, child process
trees, terminal parsers, input ordering, and durable session metadata. The
`gridbash` command remains the interactive client and owns terminal setup,
Ratatui rendering, mouse decoding, clipboard integration, and local overlays.

```text
gridbash client ── local authenticated IPC ── gridbashd
                                               ├── session A / pane PTYs
                                               ├── session B / pane PTYs
                                               └── snapshot and event journal
```

Launching `gridbash` should connect to the per-user daemon or start it on demand.
An explicit `gridbash daemon stop` performs a graceful shutdown only after the
user chooses what happens to live sessions. Client exit never implicitly kills
daemon-owned panes.

## Ownership and lifecycle

- The daemon creates every PTY and child process and remains its sole writer.
- Clients send ordered input commands tagged with session, pane, client, and
  monotonically increasing request IDs.
- The daemon assigns stable opaque IDs to sessions, tabs, panes, and generations.
  Array indexes remain a presentation detail and are never protocol identity.
- A pane generation changes after restart so late output/input acknowledgements
  cannot target the replacement process.
- The daemon owns shutdown escalation and records exit status before releasing a
  PTY. Platform-specific Job Objects/process groups remain behind one lifecycle
  interface.
- A detached session continues parsing output into bounded scrollback. It does
  not require a render loop or connected client.

## IPC and trust

Use a local per-user transport: named pipes with an owner-only ACL on Windows and
a Unix domain socket with mode `0600` on Unix. The daemon writes a discovery file
containing protocol version, endpoint, process ID, and a random boot nonce in the
user runtime directory. Never place bearer API keys or auth tokens in that file.

Every connection performs a version handshake. Mutating messages require the
boot nonce plus same-user transport credentials where the platform exposes them.
Remote TCP listeners are out of scope for the first daemon release.

Messages should use a length-prefixed, versioned envelope. JSON is acceptable for
the prototype; a schema-backed binary format can follow only if profiling shows
serialization is material. Unknown optional fields are ignored, while unknown
required capabilities fail the handshake with an actionable version error.

## State synchronization

The daemon exposes a snapshot plus ordered event stream:

1. Client requests a session snapshot at revision `N`.
2. Daemon returns layout, pane metadata, bounded screen state, and current
   revision.
3. Client subscribes from that revision.
4. Daemon sends output, title/cwd, layout, lifecycle, and status events in order.
5. If the client falls behind the bounded event buffer, the daemon requests a
   fresh snapshot instead of growing memory without limit.

Terminal screen diffs should be added only after a full-screen snapshot protocol
works correctly. Output events may be coalesced, but input acknowledgements and
lifecycle transitions must preserve order.

## Multiple clients

Multiple read-only clients are safe by default. Mutating clients use a renewable
input lease per session. The lease holder may type, resize, restart, or change
layout; other clients can request control or explicitly share it. This avoids two
terminals interleaving bytes accidentally while still allowing observers.

Client-local state includes focused pane, local scroll position, open dialogs,
palette overrides, and terminal dimensions. Daemon state includes tabs, layout,
pane labels, selected input targets only when explicitly shared, goals, and
process/session metadata.

## Persistence and recovery

Keep metadata in an atomic versioned snapshot and append-only bounded journal.
Write to a temporary file, flush, then rename over the previous snapshot. On
startup, validate the newest snapshot and fall back to the previous known-good
generation if it is corrupt.

PTY processes cannot be adopted portably after a daemon crash. A recovered record
therefore distinguishes `running`, `exited`, and `lost`. Lost panes retain bounded
history and launch metadata and offer restart; they must never be presented as
still attached.

Session records must not contain environment secrets, auth tokens, full process
environments, or manager API keys. Store references to named profiles and reload
secrets from their existing protected locations when launching a replacement.

## Configuration and compatibility

The existing user config remains the source for profiles and defaults. Daemon
startup loads and validates it, while clients may request a redacted effective
configuration. Config edits use compare-and-swap revisions so an older client
cannot overwrite newer settings.

The client and daemon advertise protocol major/minor versions and capabilities.
Major mismatches refuse mutation; compatible minor versions negotiate features.
The npm launcher should install matching client/daemon native binaries from one
package version and report both versions in diagnostics.

The current single-process mode should remain available behind an explicit
fallback during migration. Existing bounded session snapshots need a one-way,
idempotent importer; migration must copy rather than destroy the old records.

## Failure behavior

- Client disconnect: release its input lease after a short grace period; panes
  continue running.
- Slow client: coalesce output, then require resnapshot when its queue is full.
- Daemon shutdown: reject new mutations, snapshot state, gracefully terminate or
  preserve sessions according to the explicit command, then remove discovery.
- Daemon crash: clients show a disconnected state and retry with backoff; they do
  not silently start a second daemon while the old endpoint may still be live.
- Version mismatch: print installed client/daemon versions and the exact upgrade
  or restart command.
- Disk full/corrupt snapshot: keep live PTYs running, surface the persistence
  failure, and avoid overwriting the last known-good snapshot.

## Prototypes required before implementation

1. Measure named-pipe and Unix-socket throughput for 100 noisy panes with bounded
   client queues and snapshot recovery.
2. Prove owner-only endpoint discovery and peer identity on supported platforms.
3. Prototype stable screen snapshots and resubscription across terminal resize,
   alternate screen, wide characters, and scrollback.
4. Test Windows Job Object and Unix process-group behavior when the daemon or
   client crashes independently.
5. Validate the input-lease UX with two clients, including disconnect and stale
   lease recovery.
6. Exercise snapshot/journal corruption, disk-full behavior, and migration from
   existing session records.

## Decisions still open

- Whether the initial protocol uses JSON or a schema-backed encoding after the
  throughput prototype.
- Whether selections are always client-local or can become explicitly shared
  session state.
- How long detached scrollback and exited sessions are retained by default.
- Whether pane-local manager requests execute in the daemon or in a privileged
  client helper.
- Which capabilities are mandatory for the first daemon preview versus deferred
  to multi-client or remote milestones.

Implementation should not begin with a broad refactor. Land the transport and
version handshake first, then one daemon-owned PTY, snapshots, detach/reattach,
and finally multi-pane/multi-client behavior behind compatibility tests.
