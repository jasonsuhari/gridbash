# Automated pull request review

GridBash runs a repository-owned AI review agent on every non-draft pull
request when it is opened, reopened, marked ready, or updated. The agent uses
GitHub Models and the workflow's short-lived `GITHUB_TOKEN`; it does not require
a CodeRabbit account or a separately stored model API key.

## Setup

GitHub Models must be enabled under **Settings → Models → Models in this
repository**. GitHub provides included, rate-limited model usage. Paid GitHub
Models usage is separate and opt-in; the workflow does not enable billing.

The workflow uses `openai/gpt-4.1` by default. Change `REVIEW_MODEL` in
`.github/workflows/review-agent.yml` to select another model allowed by the
repository. `REVIEW_MODELS_API_VERSION` keeps the GitHub Models API contract
explicit and configurable. The checked-in `.github/copilot-instructions.md`
file defines the project-specific review standard.

To re-run a review manually:

```bash
gh workflow run review-agent.yml -f pr_number=123
```

The agent updates one marker-based comment instead of adding a new comment on
every push. A failed model request updates that same report with the error and
fails the workflow check so configuration and quota problems stay visible.
When the input budget is exceeded, the report names the truncated and omitted
files instead of implying the complete change was reviewed.

## Security model

The workflow uses `pull_request_target` so it can comment on pull requests from
forks, but it never checks out the pull request head. It checks out the exact
trusted base commit with persisted git credentials disabled. Contributor code
is not built, imported, or executed by the privileged job.

Changed-file patches are fetched through the GitHub API, bounded to 18,000
characters to fit the included model tier, and sent as explicitly untrusted data. The model has no
tools and never receives the workflow token. Generated mentions are neutralized
before the report is posted. Normal unprivileged CI remains responsible for
compiling and testing the pull request code.

## Alternatives

- **GitHub Copilot code review** provides richer native inline suggestions and
  can automatically review every new push through a repository ruleset. It
  requires an eligible paid Copilot plan or organization AI-credit policy. The
  GridBash instruction file is already compatible with it.
- **CodeRabbit** provides a polished dedicated GitHub App, inline reviews, chat,
  and managed review state. It requires installing the app and granting it
  repository access; paid-plan limits may apply.

Either service can replace or supplement this workflow later. The repository
agent is intentionally small, inspectable, vendor-light, and sufficient for an
automatic semantic pass without adding another GitHub App.
