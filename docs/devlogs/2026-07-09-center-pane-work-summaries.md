# Center pane work summaries

Date: 2026-07-09
Release target: unreleased

## Summary

- Centered the automatic agent work summary in each pane's bottom border.

## What Changed

- Kept the existing visible-terminal summarizer, which extracts the latest meaningful conversation line without calling a separate LLM endpoint.
- Applied center alignment to the pane footer title.

## Why It Matters

- The concise status reads as a pane-level caption and is easier to scan across a grid.

## Validation

- TODO

## Release Notes

- Agent-pane work summaries are centered in their footer borders.
