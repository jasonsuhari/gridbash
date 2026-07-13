# Automatic issue labeling

Date: 2026-07-13
Release target: unreleased

## Summary

- Automatically classify new GitHub issues with GridBash's existing label
  taxonomy.

## What Changed

- Added a least-privilege workflow for newly opened issues plus a manual
  recovery/backfill dispatch.
- Added a dependency-free classifier for conventional titles, issue-form area
  answers, conservative area keywords, and Windows-specific reports.
- Preserved existing labels and kept priority and resolution decisions under
  maintainer control.

## Why It Matters

- Issues created through the CLI or API now receive the same triage structure
  as issues created through repository forms.
- Maintainers get useful routing labels without losing intentional manual
  classifications.

## Validation

- npm run test:issue-labeler
- git diff --check
- GitHub Actions workflow validation and live dispatch against the tracking
  issue.

## Release Notes

- New issues now receive automatic type, triage, area, and platform labels when
  the report contains a strong matching signal.
