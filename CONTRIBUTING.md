# Contributing to GridBash

Thanks for helping improve GridBash. This guide is meant to get a useful pull request from idea to review with as little guessing as possible.

GridBash is a cross-platform Rust TUI. Changes affecting terminal behavior, process handling, keyboard input, PTY IO, packaging, or user-facing commands should be tested on Windows, Linux, and macOS when relevant.

## Fast Path

1. Search existing issues and pull requests before starting.
2. Open an issue first for large behavior changes, public API changes, config format changes, release packaging changes, or anything that may break existing workflows.
3. Fork the repository or create a branch from the latest `main`.
4. Keep the change focused. Avoid unrelated refactors in the same pull request.
5. Sign off every commit with `git commit -s`.
6. Run the relevant checks from this guide.
7. Open a pull request and fill out the pull request template.

Small fixes, documentation improvements, and focused tests can go straight to a pull request.

## Development Setup

Prerequisites:

- Windows 10 or newer, a supported Linux distribution, or macOS 13 or newer.
- Rust stable through `rustup`.
- Node.js 18 or newer for npm packaging checks.
- A compatible terminal such as Windows Terminal, Apple Terminal, iTerm2, PowerShell, Git Bash, zsh, or bash.

Install Rust on Windows:

```powershell
winget install --id Rustlang.Rustup -e
```

On macOS, install the Xcode command-line tools and Rust:

```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Clone and build:

```powershell
git clone https://github.com/jasonsuhari/gridbash
cd gridbash
cargo build
```

Run from source:

```powershell
cargo run -- 2x3 --profile powershell
```

Install the npm shim from a local checkout:

```powershell
npm install -g .
gridbash --list-profiles
```

## Validation

Run the narrowest checks that cover your change. For code changes, start with:

```powershell
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

For changes that affect packaging or the release binary, also run:

```powershell
cargo build --release
npm pack
```

If you skip a relevant check, say why in the pull request.

## Good First Contributions

Good first issues usually involve:

- README and documentation improvements.
- Reproducible bug reports with a small fix.
- Tests around parsing, layout, profile resolution, or configuration.
- Clear terminal profile detection improvements.
- Small UI state or copy fixes that do not change core PTY behavior.

Please coordinate first before working on:

- PTY lifecycle, input routing, or process shutdown behavior.
- Config file schema changes.
- CLI argument changes.
- npm packaging or release automation changes.
- Cross-platform support.
- Large UI rewrites.

## Bug Reports

Use the bug report issue template. A strong report includes:

- GridBash version or commit.
- Windows version.
- Host terminal.
- Shell or agent profile used.
- Exact command that launched GridBash.
- Expected behavior.
- Actual behavior.
- Minimal steps to reproduce.
- Relevant logs, screenshots, or terminal output.

Do not open public issues for security vulnerabilities. Follow `SECURITY.md`.

## Pull Requests

Good pull requests are small, reviewable, and explain the user-visible effect of the change.

Before opening a pull request:

- Rebase or merge from the latest `main`.
- Run the relevant validation commands.
- Confirm every commit has a DCO sign-off.
- Update docs, examples, or templates when behavior changes.
- Include screenshots, terminal transcripts, or short recordings for visible TUI changes when useful.

## Code Guidelines

- Match the existing Rust style and module boundaries.
- Prefer simple, legible code over speculative abstractions.
- Avoid new dependencies unless the problem clearly needs one.
- Keep Windows, Linux, and macOS behavior first-class.
- Treat existing config files and user workflows as compatibility surfaces.
- Put important behavior in tests when the logic can be tested without a real terminal session.

## Contribution Rights

GridBash is licensed under the MIT License. By contributing, you agree that your contribution can be distributed under the same license.

This repository uses the Developer Certificate of Origin (DCO) for routine contributions. Add a sign-off line to each commit:

```text
Signed-off-by: Your Name <you@example.com>
```

The easiest way is:

```powershell
git commit -s
```

To add a sign-off to the latest commit:

```powershell
git commit --amend -s
```

See `DCO.md` for the certification text. If you are contributing work owned by an employer or another organization, make sure you have permission before opening a pull request.

`CLA.md` contains a contributor license agreement template for larger or organization-owned contribution workflows. The project does not require separate CLA signing unless the maintainer explicitly enables a CLA workflow and asks for it.

## Community Standards

Participation in this project is covered by `CODE_OF_CONDUCT.md`. Keep discussions focused, specific, and respectful.
