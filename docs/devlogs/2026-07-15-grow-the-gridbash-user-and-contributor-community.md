# Grow the GridBash user and contributor community

Date: 2026-07-15
Release target: unreleased
Tracking issue: [#240](https://github.com/jasonsuhari/gridbash/issues/240)

## Summary

- Replaced stale Windows-only launch and roadmap copy with the current
  cross-platform product story.
- Added an operating playbook for turning outreach into activated users,
  useful feedback, and outside contributions.
- Made contributor-ready issues, Discussions, and maintainer response
  expectations easier to find from the README and issue chooser.

## What Changed

- Added `docs/OUTREACH.md` with audiences, a publication gate, a 30-day target,
  channel strategy, ethical outbound templates, a contributor funnel, and a
  lightweight campaign log.
- Rebuilt the launch kit around current Windows, Linux, and macOS support and
  stopped recommending the legacy Windows-only teaser for new campaigns.
- Updated the website, README, Rust article, roadmap, community operations, and
  v1 acceptance checklist so platform and installation claims agree.
- Documented how to handle an npm registry incident without creating replacement
  tags, weakening checks, or advertising a version the registry does not serve.
- Added direct links to Discussions, `help wanted`, and `good first issue`, plus
  a one-business-day initial response target for contributor questions.
- Opened focused workflow and contributor-introduction Discussions and curated
  five `help wanted` tasks, including two documentation-first starter issues.

## Why It Matters

- GridBash can now send prospective users to an honest installation path while
  npm publication is being resolved externally.
- Outreach has a measurable loop—trial, conversation, small fix, user story or
  contributor task—instead of optimizing only for impressions or stars.
- New contributors can tell where help is wanted, what makes a task genuinely
  approachable, and how quickly they should expect orientation.

## Validation

- `cargo fmt --check` (passed before documentation edits; no Rust files changed)
- Markdown and public-copy searches for stale platform claims
- HTML link and structure checks
- Full Rust validation deferred to the automatic integration batch under the
  repository's shared validation lease

## Release Notes

- Community and documentation update; no runtime behavior changed.
