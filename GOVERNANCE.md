# Governance

GridBash is currently maintainer-led.

## Maintainer

The project maintainer is responsible for:

- Setting project direction.
- Reviewing and merging pull requests.
- Managing releases.
- Triaging issues.
- Enforcing the code of conduct.
- Handling security reports.

## Decision Making

Small fixes can be reviewed and merged when they are correct, scoped, and validated.

Changes that affect user workflows, configuration, CLI behavior, packaging, PTY lifecycle, or long-term architecture should start with an issue or discussion before implementation.

The maintainer has final decision authority for project scope and tradeoffs.

## Becoming a Maintainer

Additional maintainers may be invited after sustained, high-quality contributions, sound review judgment, and consistent alignment with the project's code of conduct and design goals.

## Releases

Releases should include:

- A version bump where appropriate.
- A short changelog or release notes.
- Passing CI on the release branch or commit.
- A verified Windows release build when distributing binaries.
