# altaika

`altaika` is an OSS AI CLI tool for DuckDB, DuckLake, and Quack. It is built for agents that need JSON output, explicit local or remote operation boundaries, bounded source pulls, and reproducible local data workspaces.

```text
source -> explain -> pull when needed -> local DuckDB/DuckLake or remote DuckDB via Quack -> inspect or query -> manifests
```

The default is local. Remote source platform reads happen only through explicit `pull`. Remote DuckDB execution happens only through explicit `query --mode remote` or inspection commands that target a Quack endpoint.

## Install And Build

```bash
cargo build
cargo install --path .
altaika --version
altaika --help
```

Altaika embeds DuckDB through `duckdb-rs`. No DuckDB shell binary is required for core execution. `doctor` reports the embedded DuckDB runtime and whether DuckLake and Quack extensions are visible.

Local development commands:

```bash
cargo run -- doctor
cargo run -- --workspace /tmp/altaika-demo --permission auto init --local-target duckdb
cargo run -- --workspace /tmp/altaika-demo --permission auto query --engine duckdb "SELECT 1 AS ok"
```

Optional builds:

```bash
cargo build --features bigquery-adbc
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --no-default-features --features wasi
```

## Feature Policy

The default OSS build enables only the DuckDB family path:

- `duckdb`: local execution, local file scans, and DuckDB family attach.
- `ducklake`: DuckLake catalog support through DuckDB.
- `quack`: explicit remote DuckDB execution.

Everything else is opt in:

- `datafusion`: source fetch plane for remote or object store connectors.
- `adbc`: generic ADBC connector runtime.
- `bigquery-adbc`: BigQuery source connector over ADBC.
- `altertable-platform`: Altertable platform integration for Altertable owned builds.
- `wasi`: WASI build support for agent distribution experiments.

## Quick Agent Workflow

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

`explain` returns routing and safety reasoning. `plan` and `route` remain compatibility aliases, but docs use `explain`.

## Permission Modes

`--permission permission` is the default. Every response includes operation metadata under `stats.operation` and `stats.permission`.

| Mode | Agent may execute | Use for |
| --- | --- | --- |
| `permission` | `explain`, `doctor`, `inspect`, local read inspection, and `query --dry-run` | Default read only agent work |
| `auto` | Everything in `permission`, plus local writes | `init`, local `pull`, local `query` that writes manifests or tables, and `skills install` |
| `allow` | Local and remote reads or writes for this invocation | Remote Quack or remote source pulls after approval |

Blocked operations return structured `approval_required` JSON. `allow` is an explicit invocation level approval. It does not serialize secrets, bypass downstream authorization, or turn off hard validation.

## Command Reference

```bash
altaika doctor
altaika doctor --mode remote --remote quack://127.0.0.1:6544 --quack-token "$TOKEN"
altaika skills install --dry-run
altaika --permission auto skills install
altaika init --local-target ducklake
altaika explain bigquery://project.dataset.events --estimated-bytes 1048576 --estimated-rows 5000 --source-latency-ms 300
altaika pull duckdb://warehouse.duckdb/events --table events_copy --local-target ducklake --limit 1000
altaika ls --engine duckdb
altaika describe events --engine ducklake
altaika show events --engine ducklake --limit 20
altaika query --engine duckdb --mode local --file transform.sql --name transform-events
altaika query --engine duckdb --mode remote --remote quack://127.0.0.1:6544 "SELECT count(*) AS rows FROM events"
altaika --permission allow quack serve --engine duckdb --remote quack:localhost:6544 --quack-token "$TOKEN"
altaika --permission allow quack query --remote quack://127.0.0.1:6544 --quack-token "$TOKEN" "SELECT count(*) AS rows FROM events"
altaika inspect
```

`pull` writes a manifest under `.altaika/manifests/` with the source URI, source query, target table, row count, limit, and timestamp.

`query` is the SQL entrypoint. Normal `query` writes a run manifest under `.altaika/runs/`, records a SHA 256 statement fingerprint, and only stores full SQL when `--record-sql` is set. `run` remains a compatibility alias. Agent JSON output is capped by `--max-rows`, which defaults to 1000 rows and reports `rows_truncated` when more rows were available.

`query --dry-run` wraps SQL in `EXPLAIN`, skips manifest writes, and returns the query plan as JSON. It rejects statement sequences and `EXPLAIN ANALYZE` because those can execute side effects.

`ls`, `describe`, and `show` are read only inspection helpers. Use them before heavier `pull` or `query` work. `cat` remains a compatibility alias for `show`.

`explain` recommends local versus remote behavior from source scheme, observed or estimated bytes, estimated rows, and source latency. It does not auto execute remote work. `--permission auto` approves local writes, but it is not an automatic local or remote router.

## Choosing DuckDB, DuckLake, Or Quack

Use DuckLake for durable local agent work:

```bash
altaika --permission auto init --local-target ducklake
altaika --permission auto query --engine ducklake --mode local "CREATE TABLE cleaned AS SELECT * FROM events"
```

Use DuckDB for lightweight local scratch:

```bash
altaika query --dry-run --engine duckdb "SELECT 1"
```

Use Quack only for explicit remote DuckDB:

```bash
altaika explain quack://127.0.0.1:6544
altaika --permission allow quack query \
  --remote quack://127.0.0.1:6544 \
  "SELECT count(*) AS rows FROM events"
altaika --permission allow query --engine duckdb --mode remote \
  --remote quack://127.0.0.1:6544 \
  "SELECT count(*) AS rows FROM events"
```

## Quack Server Setup

Preferred local Altaika server test:

```bash
altaika --permission auto query --engine duckdb \
  "CREATE OR REPLACE TABLE events AS SELECT 1 AS id"

altaika --permission allow quack serve \
  --engine duckdb \
  --remote quack:localhost:6544 \
  --quack-token "$TOKEN"
```

The command prints one JSON readiness envelope, redacts token-like fields, and then stays in the foreground. Query it from another process:

```bash
altaika --permission allow quack query \
  --remote quack://localhost:6544 \
  --quack-token "$TOKEN" \
  --max-rows 100 \
  "SELECT count(*) AS rows FROM events"
```

If serving from another DuckDB runtime, make sure that runtime exposes `quack_serve`, `quack_query`, and `quack_stop`. `altaika doctor` reports function-level Quack readiness for the embedded runtime.

For manual DuckDB testing with a compatible DuckDB runtime:

```sql
INSTALL quack;
LOAD quack;
CALL quack_serve('quack:localhost:6544');
```

To stop it:

```sql
CALL quack_stop('quack:localhost:6544');
```

For backend or production use, bind Quack behind a reverse proxy and TLS termination. Keep `--quack-disable-ssl` for local testing or exceptional controlled environments.

## Optional BigQuery Path

BigQuery is optional and uses the ADBC feature path:

```bash
cargo build --features bigquery-adbc
altaika --permission allow pull bigquery://project.dataset.table \
  --connector adbc \
  --billing-project my-gcp-project \
  --max-bytes-billed 100000000 \
  --table local_table \
  --local-target ducklake \
  --limit 1000
```

With the feature, driver, billing project, and Google credentials available, Altaika executes the bounded BigQuery SQL through ADBC, streams Arrow batches into the local DuckDB or DuckLake target, and writes the normal pull manifest.

If setup is incomplete, Altaika returns a structured `connector_setup` error. That error includes driver readiness, Google auth hints, generated source SQL, required billing project state, and parity checks such as row count, year range, and total count checks to run after the local table exists.

## Skill Setup

Install the shipped Altaika skill for agent usage:

```bash
altaika skills install --dry-run
altaika --permission auto skills install
```

To install into a project local skills directory:

```bash
altaika --permission auto skills install --target-dir .agents/skills
```

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy -- -D warnings -D clippy::pedantic -D clippy::nursery
```

[CHANGELOG.md](CHANGELOG.md) intentionally points to GitHub Releases, following the same lightweight release-note pattern as Pup. Put release notes in GitHub Releases when publishing a version.

Optional local hooks:

```bash
pre-commit install
pre-commit install --hook-type pre-push
pre-commit run --all-files
pre-commit run --hook-stage manual --all-files
```

The regular hook stage runs cheap file hygiene plus Rust formatting and locked checks. The pre-push stage runs Rust clippy and tests. Manual hooks run the heavier release checks.

See [CONTRIBUTING.md](CONTRIBUTING.md) for contributor setup and design rules. See [SECURITY.md](SECURITY.md) for vulnerability reporting and secret handling.

Opt-in Quack smoke test:

```bash
ALTAIKA_IT_QUACK_URL=quack://127.0.0.1:6544 \
  cargo test --test cli quack_remote_smoke -- --ignored
```

See [AI CLI Best Practices](docs/ai-cli-best-practices.md), [Architecture](docs/architecture.md), and [ADRs](docs/adr/) for the command contract, permission model, performance posture, integration boundaries, and decision history.
