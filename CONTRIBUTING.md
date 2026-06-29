# Contributing

Altaika is an AI CLI for local-first DuckDB, DuckLake, and explicit Quack workflows. Keep contributions focused on deterministic command behavior, structured JSON output, and agent-safe data access.

## Setup

```bash
cargo build
cargo test
cargo run -- doctor
```

Optional checks:

```bash
cargo check --features bigquery-adbc
rustup target add wasm32-wasip2
cargo check --target wasm32-wasip2 --no-default-features --features wasi
```

## Before Opening Changes

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy -- -D warnings -D clippy::pedantic -D clippy::nursery
```

[CHANGELOG.md](CHANGELOG.md) intentionally points to GitHub Releases. Put user-visible release notes in GitHub Releases when publishing a version.

Optional local hooks:

```bash
pre-commit install
pre-commit install --hook-type pre-push
pre-commit run --all-files
pre-commit run --hook-stage manual --all-files
```

The regular hook stage runs cheap file hygiene plus Rust formatting and locked checks. The pre-push stage runs Rust clippy and tests. Manual hooks run the heavier release checks.

## Design Rules

- Use ADRs in [docs/adr](docs/adr/) for material decisions about architecture, command surface, execution engine, permissions, feature policy, release process, or developer tooling.
- Keep the OSS default focused on DuckDB, DuckLake, and Quack.
- Keep cloud warehouses, ADBC, DataFusion, and Altertable platform integrations behind opt-in features.
- Prefer the embedded `duckdb-rs` runtime over shelling out to DuckDB.
- Return machine-readable JSON for command results and failures.
- Keep remote reads, remote writes, and source pulls explicit.
- Do not add a top-level migration command. Compose `explain`, `pull`, `query`, `ls`, `describe`, `show`, and `inspect` instead.
- Do not add an OSS natural-language `ask` command until there is a deterministic provider-neutral contract.

## Secrets

Do not commit credentials, warehouse tokens, local database files, benchmark output, private plans, private specs, or machine-specific paths.
