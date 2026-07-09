# Improve SEO and discoverability

Date: 2026-07-09
Release target: unreleased

## Summary

- Added a public GitHub Pages landing page for GridBash with search-friendly metadata and structured data.
- Reworked README and package metadata around the core phrase: Windows-native terminal grid for CLI coding agents.
- Added sitemap and robots files so search crawlers have a canonical page to index.

## What Changed

- Added `docs/index.html` as a fast static landing page for `https://jasonsuhari.github.io/gridbash/`.
- Added canonical tags, Open Graph tags, Twitter card tags, SoftwareApplication JSON-LD, FAQ JSON-LD, and exact brand wording for "GridBash by Jason Suhari".
- Added `docs/sitemap.xml`, `docs/robots.txt`, and `docs/.nojekyll` for GitHub Pages.
- Updated README first-screen copy, demo links, badges, quickstart, and use-case language.
- Updated npm and Cargo metadata with stronger descriptions, homepage, keywords, and categories.

## Why It Matters

- The repo was not surfacing for the exact brand query `jasonsuhari gridbash`.
- GitHub Pages gives GridBash a dedicated indexable URL with stable title, description, canonical URL, social card, sitemap, and visible owner/project wording.
- Consistent wording across GitHub, npm, Cargo metadata, and the landing page gives search engines clearer signals about what GridBash is.

## Validation

- Ran `cargo test`.
- Ran `npm pack --dry-run --ignore-scripts`.
- Checked repository status before committing.

## Release Notes

- Improved public discoverability with an SEO-focused GitHub Pages landing page and refreshed repository/package metadata.
