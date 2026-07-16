# GridBash User and Contributor Growth Playbook

This is the operating playbook for growing GridBash without turning the project
into a stream of generic launch posts. The goal is to help the right developers
try a real workflow, learn from them quickly, and make useful contributions
approachable.

## Thirty-Day Outcomes

The first growth milestone is a small, active community:

- 20 people who launch a real GridBash agent grid.
- 8 direct conversations about first-use friction.
- 5 people who report using GridBash more than once.
- 3 outside contributors who claim an issue.
- 2 merged pull requests from outside contributors.
- A first maintainer response to contributor questions within one business day.

Stars, page views, downloads, and clones are useful reach signals, but they are
not activation or retention. npm downloads and repository clones can also
include automation.

An **activated user** has launched at least a two-pane agent grid and used
focused or selected-pane input. A **returning user** reports using GridBash in a
second work session. Until the project has a justified, privacy-preserving
analytics design, measure both through conversations, Discussions, and a small
campaign log instead of adding product telemetry.

## Positioning

Lead with the workflow rather than the implementation:

> GridBash is a managed terminal workspace for developers running multiple CLI
> coding agents. Keep every session visible, route prompts intentionally, and
> isolate parallel work in git worktrees.

For contributors:

> Help build a cross-platform Rust PTY and TUI at the intersection of terminals,
> developer tooling, and multi-agent workflows.

The playful `tokenmaxx` line can follow the concrete description. Technical and
professional audiences should understand the product before encountering the
in-joke.

## Audiences

### Agent power users

Developers already running two or more Codex, Claude, Gemini, Aider, OpenCode,
Goose, Amp, Cursor, or Copilot CLI sessions. Show implementation/review/test
loops, comparisons, and worktree isolation.

### Terminal and Rust builders

People interested in PTYs, terminal emulation, Ratatui, process lifecycle,
cross-platform packaging, and developer tools. Lead with technical lessons and
specific contribution tasks rather than AI marketing.

### Adjacent maintainers and educators

Maintainers of agent CLIs, terminal tools, developer newsletters, meetups, and
technical channels. Offer a useful integration example, technical write-up, or
demo. Do not ask for an unearned endorsement.

## Publication Gate

Do not direct broad traffic into a broken installation path. Before each public
campaign:

1. Confirm the `main` CI badge is green.
2. Compare `npm view gridbash version` with the latest stable GitHub release.
3. Confirm every advertised platform has an artifact in that release.
4. Smoke-test the exact install and first-run commands used in the post.
5. Open every destination link in a signed-out browser session.

If npm temporarily trails the GitHub release, say so plainly, link testers to
the matching GitHub artifacts, and postpone broad launch posts. Do not create
new tags or repeated releases just to work around a registry incident.

## The Growth Loop

```text
useful proof -> targeted trial -> first-use conversation -> small fix
     ^                                                   |
     +--------- user story or contributor task <--------+
```

Every post or message should have one audience, one demonstrated workflow, and
one request. Good requests include:

- “Try this two-pane workflow and tell me where setup becomes unclear.”
- “What information would you need before trusting worktree isolation?”
- “This test task is scoped and mentored; comment if you want to take it.”

“Please support the project” and “please star this” do not produce useful
learning.

## Channel Plan

| Channel | Primary job | Best material | Call to action |
| --- | --- | --- | --- |
| X and LinkedIn | Recruit first users | Native 10–20 second workflow video | Try one exact command |
| Show HN | Reach technical early adopters | Personal build story plus runnable project | Critique routing and isolation |
| This Week in Rust | Find Rust users and contributors | Tooling update and one mentored task | Claim the linked issue |
| Agent communities | Reach existing multi-agent users | Agent-specific recipe | Share current workflow friction |
| Rust and terminal communities | Build technical credibility | PTY, rendering, or worktree article | Review the design or code |
| Product Hunt | Broader awareness after proof | Polished demo and user evidence | Try the stable release |

Read each community's current rules before posting. Adapt the explanation to the
community instead of copying the same announcement everywhere.

## Weekly Cadence

- **Monday — proof:** publish one short, outcome-first workflow clip.
- **Tuesday — conversations:** invite up to five relevant people to a focused
  trial and follow up with active testers.
- **Wednesday — depth:** publish one technical lesson, devlog, or architecture
  note that stands on its own.
- **Thursday — contribution:** highlight one contributor-ready task and offer
  orientation.
- **Friday — learning:** record source, conversations, activations, recurring
  friction, issues claimed, and changes shipped.

One strong artifact can be adapted for several channels over a week. Do not
publish every variation on the same day.

## Ethical Outbound

Start with people who have publicly discussed multi-agent CLI workflows or have
already engaged with GridBash. Send at most ten well-researched invitations per
week. Do not scrape addresses, bulk-message stargazers, automate replies, or ask
for coordinated votes.

User trial invitation:

```text
I saw your post about running multiple <AGENT> sessions. I am building
GridBash, a terminal workspace that keeps those sessions visible and can put
each one in its own git worktree.

Would you be willing to try one 10-minute workflow? I am looking for setup and
input-routing friction, not a promotional post. The exact command is:

    gridbash 2x2 --profile <PROFILE> --worktrees

If it is not relevant to your workflow, no reply needed.
```

Maintainer or educator invitation:

```text
I maintain GridBash, an MIT-licensed Rust terminal workspace for parallel CLI
agents. I wrote a practical explanation of <TECHNICAL TOPIC> while building it.
It may fit your audience because <SPECIFIC REASON>.

Here is the article/demo: <LINK>. No expectation to share it; technical
corrections would be genuinely useful.
```

## Contributor Funnel

Keep three to five issues actively ready for outside contributors. Each should
have `help wanted`; reserve `good first issue` for tasks that truly require no
hidden architecture or product decision.

Every promoted task needs:

- one clear outcome and a short explanation of why it matters;
- the likely files or modules involved;
- acceptance checks and an exact focused validation command;
- size or expected scope;
- a note that the maintainer will help with orientation;
- no unresolved design decision disguised as implementation work.

Contributor response practice:

1. Acknowledge a claim or question within one business day.
2. Confirm scope before the contributor invests significant work.
3. Keep review feedback specific and separate required changes from ideas.
4. Credit contributors in release notes and the next relevant announcement.
5. Invite a second contribution only after the first experience is complete.

Use GitHub Discussions for open-ended workflow and design questions. Convert a
discussion into an issue only after the outcome is concrete enough to build.

Suggested recurring prompts:

- “What does your multi-agent terminal workflow look like today?”
- “Which GridBash setup step was hardest on your OS and terminal?”
- “New contributors: which part of PTY, TUI, packaging, or docs interests you?”

The current seed conversations are the
[multi-agent workflow discussion](https://github.com/jasonsuhari/gridbash/discussions/256)
and the
[new-contributor introduction](https://github.com/jasonsuhari/gridbash/discussions/257).

## First Campaign

1. Clear the publication gate and make the current platform story consistent.
2. Demonstrate an implementation/review/test loop in three isolated worktrees.
3. Invite ten relevant developers to try that exact workflow.
4. Publish the same proof natively on X and LinkedIn on separate days.
5. Run Show HN only after the install path is healthy and remain available to
   answer questions.
6. Submit one technical project update and one mentored issue to This Week in
   Rust.
7. Ship the most common first-use fix and publish what changed because of user
   feedback.

## Campaign Log

Record one row per meaningful activity. Keep personal contact details out of the
repository.

| Date | Source | Artifact or conversation | Trials | Activations | Returning users | Issues claimed | Learning or action |
| --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| YYYY-MM-DD | example | workflow demo | 0 | 0 | 0 | 0 | Replace with result |

Review the log weekly. Continue a channel when it produces activated users,
useful conversations, or contributors—not merely impressions.
