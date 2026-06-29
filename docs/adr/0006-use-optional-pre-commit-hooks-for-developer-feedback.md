# 0006. Use Optional Pre-Commit Hooks For Developer Feedback

## Status

Accepted

## Context

AI agents and humans both benefit from fast local feedback. Rust projects commonly rely on `cargo fmt`, `cargo check`, `cargo clippy`, and `cargo test`. Apache Arrow and DataFusion also keep repository-level developer tooling that runs formatting and lint checks locally.

Local hooks should reduce avoidable CI failures, but CI remains the source of truth.

## Decision

Altaika ships an optional `.pre-commit-config.yaml`.

The regular pre-commit stage runs general file hygiene plus quick Rust checks. The pre-push stage runs Rust clippy and tests. Manual hooks run heavier Rust checks such as locked build, strict clippy, BigQuery ADBC feature check, and WASI target check.

## Consequences

Contributors can install hooks with `pre-commit install` and `pre-commit install --hook-type pre-push`. Hooks are helpful but not mandatory for using the project. CI must continue to run the authoritative verification commands.
