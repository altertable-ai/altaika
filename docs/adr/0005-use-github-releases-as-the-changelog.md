# 0005. Use GitHub Releases As The Changelog

## Status

Accepted

## Context

Altaika is early OSS. Pup uses a lightweight repository changelog that points readers to GitHub Releases. That keeps release notes close to released artifacts instead of duplicating history in the repository.

## Decision

`CHANGELOG.md` remains a minimal pointer to GitHub Releases.

## Consequences

Per-release notes are written in GitHub Releases. Pull requests and commits should not expand `CHANGELOG.md` with unreleased notes unless this ADR is superseded. Release automation can later generate GitHub Release notes from commits or tags.
