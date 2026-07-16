# GridBash v1.0 Acceptance Checklist

This checklist defines the minimum evidence required to call the Windows build
of GridBash stable. A release candidate may ship before every box is checked,
but the stable `v1.0.0` tag must not.

Record evidence beside each item in the release PR or linked test report. A
passing check from an older commit does not qualify unless the release commit is
an ancestor of that tested commit and no relevant code changed afterward.

## Installation and launch

- [ ] `npm install -g gridbash` succeeds for a clean Windows x64 user without a
      Rust toolchain.
- [ ] `gridbash --version` reports the npm package version and launches the
      packaged native executable rather than a repository worktree.
- [ ] Bare launch discovers available agents and shells, leads with managed
      agent-workspace setup, and labels raw terminals as a secondary path.
- [ ] Workspace setup can choose profile, compatible auth, project, dimensions,
      and worktree isolation without globally replacing agent commands.
- [ ] PowerShell, PowerShell 7, cmd, and Git Bash invocation inheritance each
      select the expected pane shell unless an explicit profile overrides it.
- [ ] Paths containing spaces and non-ASCII characters work for installation,
      config, current directories, and managed worktrees.

## PTY and process lifecycle

- [ ] A pane can launch, accept keyboard and pasted input, resize, produce ANSI
      output, report its cwd, and exit without corrupting sibling panes.
- [ ] Closing GridBash terminates every owned child process tree without taskkill
      success spam or orphaned descendants.
- [ ] Restarting an exited pane preserves its profile, cwd, label, auth choice,
      and managed-worktree metadata.
- [ ] A 10x10 grid can start and shut down cleanly; busy output does not make
      focus movement, selection, or quit controls unusable.
- [ ] Unicode input/output, OSC 52 copy, cursor queries, alternate-screen apps,
      and xterm mouse reporting have regression coverage.

## Profiles and configuration

- [ ] `gridbash --list-profiles` clearly identifies available, missing, and
      selected profiles without exposing credentials or unnecessary user paths.
- [ ] Built-in Windows profiles resolve expected executables, and a custom
      profile reports an actionable error when its executable is missing.
- [ ] Config defaults remain backward compatible; unknown legacy tables do not
      prevent startup.
- [ ] Settings that claim persistence survive restart, while session-only
      controls are labeled as session-only.
- [ ] Claude and Codex auth profile selection never prints tokens and keeps
      account labels masked.

## Grid interaction

- [ ] Focus movement wraps predictably and skips sleeping panes where required.
- [ ] Single-pane input, selected multi-pane input, and select-all behavior route
      bytes only to the intended live panes.
- [ ] Drag selection and wheel scrolling stay inside the pane under the pointer.
- [ ] Rename, swap, sleep/wake, grid resize, tabs, command bar, pane settings,
      help, and recovery dialogs work at both 80x24 and 160x50.
- [ ] Every modeless GridBash shortcut is discoverable in the in-app help and
      documented in the README.

## Sessions and worktrees

- [ ] Session snapshots are bounded, recover from a truncated/corrupt newest
      record, and never store credentials.
- [ ] `gridbash resume --list`, `--latest`, and unique-id selection restore grid,
      profile, cwd, label, auth, and recent-history metadata without replaying
      commands automatically.
- [ ] Managed worktrees are created inside the configured repository, use unique
      branch names, and are not deleted when GridBash exits.
- [ ] Launch or cleanup failures identify the affected pane/worktree and leave
      existing user branches untouched.

## Documentation and support

- [ ] README install, quickstart, controls, config, profile, session, and local
      development instructions match the release candidate.
- [ ] `--help`, example config, release documentation, and npm package contents
      agree on supported Windows versions and profiles.
- [ ] Security reporting, support, contribution, license, and code-of-conduct
      links are present and valid.
- [ ] Known v1 limitations explicitly include single-client background hosts and
      the lack of a consolidated, multi-client session daemon.

## Packaging and release

- [ ] `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test`, launcher tests, and a release build pass on the release commit.
- [ ] CI builds and packs Windows x64 from a clean runner, and the produced npm
      tarballs contain only the expected launcher, metadata, docs, and executable.
- [ ] Native packages publish before the root launcher; exact versions match
      across Cargo, npm root, optional native package, tag, and GitHub release.
- [ ] npm trusted publishing/provenance and the token fallback are tested or
      explicitly waived with a documented reason.
- [ ] A clean-machine smoke test installs from the intended npm dist-tag, launches
      a real pane, exercises input/resize/quit, and removes the package cleanly.

## Stability decision

The v1.0 release PR must link the completed evidence, list every waived item,
and identify an owner and follow-up issue for each waiver. Any failure involving
data loss, credential exposure, orphaned process trees, unusable input, or an
uninstallable package blocks the stable tag.
