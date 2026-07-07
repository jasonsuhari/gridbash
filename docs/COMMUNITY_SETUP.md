# Community Setup Notes

This document records the contribution infrastructure added to GridBash and the repository settings that still need maintainer access on GitHub.

## Research Basis

The setup follows GitHub's community health model: a public repository is easier to evaluate when it has visible README, license, contributing, code of conduct, support, security, and issue template files in supported locations.

GitHub surfaces `CONTRIBUTING.md` when people open issues or pull requests, and also shows it in repository navigation. Issue and pull request templates reduce maintainer back-and-forth by asking for the details reviewers need before the conversation starts.

For contribution rights, this repo starts with DCO sign-offs because they are lightweight and familiar in open source. A separate CLA is kept as an optional template because CLAs can be useful for organizations, but they add more contributor friction and should be reviewed before enforcement.

## What Is Now In The Repository

- `CONTRIBUTING.md` for contributor onboarding, setup, validation, issue guidance, pull request expectations, and DCO instructions.
- `CODE_OF_CONDUCT.md` for community expectations and enforcement.
- `SECURITY.md` for private vulnerability reporting.
- `SUPPORT.md` for usage help routing.
- `GOVERNANCE.md` for maintainer-led project governance.
- `DCO.md` plus `.github/workflows/dco.yml` to require commit sign-offs on pull requests.
- `CLA.md` as a CLA template if separate CLA enforcement is later needed.
- `.github/ISSUE_TEMPLATE/` issue forms for bugs, features, and questions.
- `.github/pull_request_template.md` for consistent review context.
- `.github/CODEOWNERS` to route reviews to the maintainer when branch protection is enabled.

## Recommended GitHub Settings

Enable these in repository settings when you are ready:

- Issues: enabled.
- Discussions: optional, useful once repeated support questions appear.
- Private vulnerability reporting: enabled under Security settings.
- Branch protection for `main`:
  - Require the `CI / windows` check.
  - Require the `DCO / Signed-off-by` check.
  - Require pull request review before merging.
  - Require conversation resolution before merging.
  - Block force pushes.

## Suggested Labels

GitHub's default labels are enough to start. Add these when triage volume grows:

- `needs triage`
- `good first issue`
- `help wanted`
- `windows`
- `terminal`
- `pty`
- `packaging`
- `docs`
- `security`

## Good First Issue Practice

A good first issue should have:

- A single clear outcome.
- Reproduction steps or an exact file area.
- A suggested validation command.
- No hidden product decision.
- No expected knowledge of GridBash internals beyond the files named in the issue.

## CLA Activation

GridBash currently uses DCO sign-offs because they are lighter for contributors than a separate CLA.

If a separate CLA becomes necessary:

1. Have qualified counsel review `CLA.md`.
2. Publish the final CLA text somewhere stable, such as a GitHub Gist used only for the CLA.
3. Link the repository or organization to a CLA tool such as CLA Assistant.
4. Add the CLA status check to branch protection.
5. Update `CONTRIBUTING.md` and `.github/pull_request_template.md` to state that CLA signing is mandatory.

Do not require both DCO and a CLA unless there is a clear legal reason. That adds friction for new contributors.
