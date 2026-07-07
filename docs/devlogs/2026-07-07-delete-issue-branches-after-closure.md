# Delete Issue Branches After Closure

Date: 2026-07-07
Release target: unreleased

## Summary

- Added issue-close automation that removes clearly related remote branches.

## What Changed

- Added a GitHub Actions workflow that runs when an issue is closed.
- The workflow scans remote branches and matches conservative issue naming patterns such as `issue-53`, `gh/53`, and `fix/53-short-description`.
- Matching branches are deleted only when they are not the repository default branch and are not protected.

## Why It Matters

- Closed issue branches no longer accumulate after work is finished, while protected and unrelated branches stay untouched.

## Validation

- Ran `git diff --check`.
- Checked the embedded workflow JavaScript with `node`.
- Ran a `node` sanity check for matching expected issue branch names and rejecting obvious false positives.
- Attempted `npx --yes actionlint@latest .github\workflows\delete-issue-branches.yml`, but npm could not determine an executable for that package.

## Release Notes

- Closed issues now trigger cleanup for unprotected remote branches that are clearly named after the issue number.
