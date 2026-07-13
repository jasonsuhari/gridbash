# GridBash code review instructions

Review pull requests as a skeptical senior maintainer. Report only actionable
problems introduced or exposed by the change; do not block on personal style
preferences or restate the diff.

Prioritize these areas:

- Correctness, data loss, hangs, races, resource leaks, and broken error paths.
- Trust boundaries around PTY input/output, local control APIs, credentials,
  session data, GitHub Actions tokens, and contributor-controlled content.
- Windows, macOS, and Linux behavior, including shell resolution, process
  lifecycle, path handling, terminal capabilities, and native packaging.
- Compatibility of CLI flags, TOML configuration, saved sessions, npm package
  layout, and existing user workflows.
- Ratatui layout behavior in small terminals and keyboard handling conflicts.
- Missing regression tests for behavior that can be exercised deterministically.

Use these severities:

- `P0`: immediate security compromise, destructive data loss, or universally
  broken release.
- `P1`: likely user-facing defect, hang, major regression, or meaningful
  security weakness that should block merge.
- `P2`: narrower correctness or maintainability defect worth fixing before
  merge when practical.

For every finding, name the affected file and line or changed hunk, explain the
concrete failure scenario, and suggest the smallest useful correction. If there
are no findings, say so plainly and mention any validation gap that remains.
