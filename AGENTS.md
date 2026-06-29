# AGENTS.md

These instructions apply to the whole repository.

Use ADRs in [docs/adr](docs/adr/) for material decisions. Add or update an ADR when changing architecture, command naming, execution engine, permissions, feature policy, release process, or developer tooling.

## Purpose

Altaika is an AI CLI tool only. Optimize every command, output, and document for AI agents, not for a general human SQL shell. It runs local DuckDB or DuckLake through `duckdb-rs` by default, uses Quack for explicit remote DuckDB sessions, pulls bounded source data explicitly, runs SQL, and records manifests.

## Direction

- `explain` must explain the local or remote execution choice before work crosses a source boundary. `plan` and `route` exist only as compatibility aliases.
- Every command response must attach `skill`, `mode`, `stats`, `stats.operation`, and `stats.permission` so an AI agent can decide what happened without parsing prose.
- SQL query responses must honor the agent row cap. Keep `--max-rows` bounded by default, return `rows_truncated`, and ask for explicit user approval before raising caps on large or remote result sets.
- The default `--permission permission` mode allows explaining, local read inspection, `doctor`, `inspect`, and `query --dry-run`. Local writes require `--permission auto`. Remote reads and writes require `--permission allow`.
- `--permission auto` is an approval policy for local writes, not automatic local or remote route switching.
- `doctor` is read only. It should report the embedded `duckdb-rs` runtime, DuckLake, Quack, feature, and optional remote Quack readiness without mutating the system.
- `skills install` is a local write unless `--dry-run` is set. Keep installed skills generic and free of machine specific paths.
- `ls`, `describe`, and `show` are read only inspection helpers. They should not write run manifests. `cat` remains an alias for `show`.
- `query --dry-run` must use non-executing `EXPLAIN`, skip manifests, keep read only permissions, and reject statement sequences or `EXPLAIN ANALYZE`.
- Normal `query` writes a manifest, so it is a local write in local mode even when the SQL starts with `SELECT`. `run` remains an alias for `query`.
- Remote DuckDB execution must go through explicit Quack settings. Do not hide remote execution behind local commands.
- `quack serve` starts a foreground Quack server and requires `--permission allow`. Redact token-like fields from readiness output.
- `quack query` is the terminal-like remote SQL entrypoint and should not write local manifests.
- Use `--local-target` in docs and examples for local DuckDB or DuckLake targets. Keep `--target` only as a compatibility alias.
- `pull` is the source boundary for remote systems such as BigQuery, Snowflake, Databricks, and Altertable.
- BigQuery ADBC is optional. When enabled, it should execute bounded SQL through ADBC, stream Arrow batches into local DuckDB or DuckLake, write a pull manifest, and return structured setup errors for missing driver, billing project, or Google credentials.
- DuckDB and DuckLake are the primary local runtime and storage targets.
- Local DuckDB and DuckLake table sources are pull sources, not a separate migration command.
- DataFusion belongs in the source and fetch layer when it improves remote connectors, Arrow streaming, projection, filtering, or pushdown.
- Do not route local CSV or Parquet through DataFusion when DuckDB can scan it directly.
- Quack is part of the default OSS feature set for explicit remote DuckDB transport, not the default runtime.
- Altertable usage must stay optional for the OSS core. Treat it as a source connector, publish target, validation target, or semantic metadata target.
- Python is useful for examples and connector experiments, not for the core CLI.
- Migration can be achieved by composing `explain`, `pull`, `query`, inspection commands, and `inspect`, but it must not become a top-level command or the OSS product identity.
- Do not add an OSS `ask` command until there is a deterministic provider-neutral contract. Natural language to SQL belongs outside the core CLI for now.
- Keep `CHANGELOG.md` minimal as a pointer to GitHub Releases. Put per-release notes in GitHub Releases instead of expanding the repository changelog.
- Keep `CONTRIBUTING.md` and `SECURITY.md` generic. Do not add Altertable-only assumptions to OSS contributor guidance.

## Checks

Run these before claiming the repo is ready:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

For strict Rust review:

```bash
cargo clippy -- -D warnings -D clippy::pedantic -D clippy::nursery
```

If `pre-commit` is installed, run the optional local hooks:

```bash
pre-commit install
pre-commit install --hook-type pre-push
pre-commit run --all-files
pre-commit run --hook-stage manual --all-files
```

The hooks combine general file hygiene with Rust Cargo gates. Do not treat them as a replacement for CI.

## Do Not Commit

Do not include benchmark output, private plans, private specs, credentials, or machine-specific paths.

## Secrets

Do not store warehouse credentials in manifests or config files. Remote connectors must use runtime environment variables, platform-native credential providers, or explicit user-provided profiles that do not serialize secrets.
