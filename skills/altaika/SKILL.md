---
name: altaika
description: Use Altaika for AI-only local-first data workflows with DuckDB, DuckLake, Quack, JSON outputs, explicit permissions, and manifests.
metadata:
  version: "0.1.0"
  repository: https://github.com/altertable-ai/altaika
  tags: ai-cli,duckdb,ducklake,quack,datafusion,adbc,bigquery
---

# Altaika

Altaika is an AI CLI tool. Use it when an agent needs to inspect data, make a bounded local copy, run SQL in a local DuckDB or DuckLake workspace, or explicitly query a remote DuckDB through Quack.

## Quick Start

```bash
altaika doctor
altaika --permission auto init --local-target ducklake
altaika explain csv://events.csv
altaika --permission auto pull csv://events.csv --table events --local-target ducklake --limit 1000
altaika ls --engine ducklake
altaika describe events --engine ducklake
altaika show events --engine ducklake --limit 20
altaika query --dry-run --engine ducklake "SELECT count(*) AS rows FROM events"
altaika --permission auto query --engine ducklake "CREATE TABLE signups AS SELECT * FROM events WHERE event_name = 'signup'"
altaika inspect
```

## Permission Modes

- `permission`: default. Allows explaining, local read inspection, `doctor`, `inspect`, and `query --dry-run`.
- `auto`: allows local writes such as `init`, local `pull`, normal local `query`, and `skills install`.
- `allow`: explicit approval for remote reads and writes such as Quack or optional remote source pulls.

If a command returns `approval_required`, inspect `stats.permission`, then rerun with the narrowest acceptable permission mode.

## Command Decision Table

| Need | Command |
| --- | --- |
| Check local DuckDB, DuckLake, Quack, or feature readiness | `altaika doctor` |
| Install this skill | `altaika --permission auto skills install` |
| Decide local versus remote before touching data | `altaika explain <source-uri>` |
| Make a bounded local copy | `altaika --permission auto pull <source-uri> --table <table>` |
| List tables | `altaika ls --engine ducklake` |
| Inspect schema | `altaika describe <table> --engine ducklake` |
| Preview rows | `altaika show <table> --engine ducklake --limit 20` |
| Explain SQL without manifest writes | `altaika query --dry-run --engine ducklake "<sql>"` |
| Execute local SQL and write a manifest | `altaika --permission auto query --engine ducklake "<sql>"` |
| Start a local Quack server | `altaika --permission allow quack serve --engine duckdb --remote quack:localhost:6544` |
| Query remote DuckDB through Quack without a local manifest | `altaika --permission allow quack query --remote quack://host:port "<sql>"` |
| Execute remote DuckDB through Quack | `altaika --permission allow query --engine duckdb --mode remote --remote quack://host:port "<sql>"` |
| See local state | `altaika inspect` |

## Local Vs Remote Runtime Choice

- Prefer local DuckLake for durable agent work, local history, and repeated transformations.
- Prefer local DuckDB for lightweight scratch queries.
- Prefer `pull` before repeated analysis of remote platforms such as BigQuery, Snowflake, Databricks, PostgreSQL, or Altertable.
- Prefer Quack only when the target is already a remote DuckDB session and remote execution is intentional.
- Do not use DataFusion for local CSV or Parquet when DuckDB can scan directly.
- Use `explain` for route recommendations. It uses stats, but it does not auto execute remote work.

## Output Contract

Every command returns JSON with:

- `schema_version`
- `skill`
- `mode`
- `stats`
- `data` or `error`

Use `stats.operation`, `stats.permission`, `rows_returned`, `rows_truncated`, `row_limit`, `row_count`, `remote_calls`, `statement_sha256`, and manifest paths for follow-up decisions. Do not parse prose.

## Common Pitfalls

- Use `explain`, not `plan` or `route`, in new examples. `plan` and `route` are only aliases.
- Normal local `query` writes a manifest, so it requires `--permission auto`.
- Query output is capped by `--max-rows`. Ask before raising caps on large or remote result sets.
- Use `query --dry-run` for read-only `EXPLAIN`. Do not pass statement sequences or `EXPLAIN ANALYZE`.
- Keep secrets out of SQL files and manifests. Altaika records SQL hashes by default.
- BigQuery is optional. Use `--connector adbc` only when the binary is built with `bigquery-adbc`, the driver is installed, a billing project is available, and Google auth is configured. On success Altaika streams ADBC Arrow batches into local DuckDB or DuckLake and writes a pull manifest. On setup errors, inspect `stats.auth`, `stats.driver`, `stats.phase_error`, and `stats.migration_goal.parity_checks`.
- Prefer `quack query` for terminal-like remote SQL without a local manifest. Prefer `query --mode remote` when a local run manifest is useful.
- Use `--quack-disable-ssl` only for local or controlled testing.
