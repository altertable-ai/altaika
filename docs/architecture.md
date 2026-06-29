# Architecture

Altaika is an AI CLI tool for DuckDB, DuckLake, and Quack. It is local-first, supports DuckLake as a first-class target through DuckDB, and uses Quack for explicit remote DuckDB sessions.

Architecture decisions are recorded under [docs/adr](adr/). Add or update an ADR for material changes to command surface, execution engine, feature policy, permissions, changelog/release process, or developer tooling.

## Core Flow

```text
source -> explain -> pull when needed -> local DuckDB/DuckLake or remote DuckDB via Quack -> inspect or query -> validate -> publish
```

`query` executes against local DuckDB or DuckLake, or against remote DuckDB through Quack. `pull` is the explicit remote data-source boundary where cost, credentials, and platform semantics are involved. `run` remains a compatibility alias for `query`.

## Routing Model

The CLI should pick the cheapest understandable plan, then explain it in JSON:

- Local CSV and Parquet: DuckDB scanners. No DataFusion hop is needed.
- Local DuckDB and DuckLake tables: read-only attach, bounded copy, and manifest.
- Object stores: DuckDB scanners or DataFusion object-store readers, depending on credentials, file layout, and pushdown needs.
- Remote platforms: DataFusion or a source-specific connector for bounded Arrow fetches, then DuckDB or DuckLake locally.
- Remote DuckDB sessions: Quack only when the agent selects that route explicitly.

This keeps the CLI useful for AI agents: they can ask for an explanation before they spend money, expose credentials, or refresh a local copy. Explain output should include:

- recommendation and confidence
- missing decision signals
- estimated or observed data size
- estimated rows
- estimated source latency
- size and latency classes
- compiled feature flags and the feature required by the source route
- thresholds used by the decision
- OSS core versus optional integration boundaries

## Cargo Features

The default OSS feature set is intentionally narrow:

- `duckdb`: local execution, local file scans, and DuckDB-family attach.
- `ducklake`: DuckLake catalog support through DuckDB.
- `quack`: explicit remote DuckDB execution.

Connector and platform features are opt-in:

- `datafusion`: source fetch plane for remote or object-store connectors.
- `adbc`: generic ADBC connector runtime.
- `bigquery-adbc`: BigQuery source connector over ADBC.
- `altertable-platform`: Altertable platform integration for Altertable-owned builds.
- `wasi`: WASI build support for agent distribution experiments.

## Engine Strategy

Do not put DataFusion on top of DuckDB by default. That makes the stack harder to explain and usually adds an adapter layer before we know it improves the agent workflow.

Use peer engines behind the SQL entrypoint:

- DuckDB local: scratch SQL, local files, fast execution.
- DuckLake local: durable catalog and multi-step local work, reached through DuckDB.
- DuckDB remote through Quack: explicit remote DuckDB execution.
- DataFusion: source fetch engine when it improves connector behavior, Arrow streaming, projection, filtering, or source pushdown.

DataFusion should not be forced into local CSV or Parquet reads when DuckDB can scan those files directly and write the local table in one step. DataFusion is valuable when it can read or federate sources and hand Arrow/Parquet data to the local DuckDB or DuckLake workspace.

## Quack Runtime Shape

`quack serve` uses the embedded `duckdb-rs` runtime to expose a local DuckDB or DuckLake workspace as a foreground Quack server. It prints a single readiness JSON envelope, redacts token-like fields, and then stays alive until the process is stopped.

Backend production should run the same shape under process supervision, TLS termination, and token management. A backend-owned DuckDB runtime must expose `quack_serve`, `quack_query`, and `quack_stop`; `doctor` reports these functions for the embedded runtime.

`quack query` is the terminal-like remote SQL command and does not write a local manifest. `query --mode remote` remains available when a local run manifest is useful.

## Rust Boundaries

The Rust layout should follow the same separation as mature Arrow/DataFusion projects such as ROAPI, but adapted for a CLI instead of an HTTP service:

- `cli`: command contract, engine names, mode names, and shared input structs.
- `agent`: stable JSON envelope for AI agents.
- `error`: user-facing error modes, skills, and stats.
- `features`: compiled feature metadata returned by `explain`.
- `permissions`: operation classes and approval summaries.
- `connectors`: optional source connectors such as BigQuery ADBC.
- `duckdb_runtime`: embedded `duckdb-rs` execution and JSON row conversion.
- `lib`: command dispatch, orchestration, and the reusable `altaika::run` entrypoint.
- `main`: thin binary wrapper around `altaika::run_cli`.

The next split should move planning and skill installation into dedicated modules before adding more source engines. Do not add a Cargo workspace split until the module boundaries stop moving.

## Library Usage

The package exposes both a binary and a Rust library. Local tools can call `altaika::run(cli)` when they already have a parsed `Cli`, while normal agent usage should keep calling the binary:

```bash
cargo run -- doctor
cargo run -- --workspace /tmp/altaika-demo --permission auto init --local-target duckdb
cargo run -- --workspace /tmp/altaika-demo --permission auto query --engine duckdb "SELECT 1 AS ok"
```

Altertable backend should not link Altaika or shell out to it for its normal tests. The backend already owns a DuckDB stack through `altertable-duckdb` and an Altertable fork of `duckdb-rs`. Use Altaika as an external black-box client only in ignored integration tests against a local Quack endpoint or backend API.

## Performance Rules

The default binary should stay small and fast to start. Keep DuckDB, DuckLake, and Quack in the default feature set. Keep DataFusion, ADBC, BigQuery, and Altertable platform code opt-in.

Fast paths should avoid unnecessary engine hops:

- Local CSV, Parquet, DuckDB, and DuckLake should stay on the DuckDB-family path unless a benchmark proves DataFusion is faster for a specific workload.
- Remote platform reads should be bounded, projected, and filtered before localizing data.
- Repeated agent work should reuse the local workspace and manifests instead of re-reading remote sources.
- JSON responses should remain bounded. Large result sets belong in DuckDB or DuckLake tables, not in agent tokens.
- Embedded DuckDB through `duckdb-rs` is the default runtime. Measure build time, startup time, and repeated command behavior before adding more native features to the default build.

## Source Readers

Other engines can help read sources, but they should act as readers or translators into the local workspace:

```text
source engine -> DataFusion/source connector/other reader -> Arrow or Parquet -> DuckDB/DuckLake local target
```

This keeps the CLI generic. The local DuckDB or DuckLake workspace becomes the validation point, while engine-specific code remains replaceable. Migration can be achieved by composing the primitives, but it is not a CLI command.

## Integration Boundaries

The OSS core should stay usable without DataFusion, ADBC, or Altertable credentials. Its required capabilities are route explanations, local DuckDB and DuckLake workspaces, manifested pulls, SQL execution through `query`, workspace inspection, and explicit Quack remote execution.

Optional layers are separate:

- DataFusion: remote source fetch plane for connector implementations.
- Altertable: optional source connector, publish target, validation target, or semantic metadata target.

This separation keeps the OSS CLI useful by itself while still making commercial integrations possible.

## DuckDB And DuckLake

DuckDB is the execution engine. DuckLake is a first-class catalog target reached through DuckDB's DuckLake extension:

```sql
INSTALL ducklake;
LOAD ducklake;
ATTACH 'ducklake:.altaika/lake.ducklake' AS lake;
USE lake;
```

The implementation uses `duckdb-rs` for embedded DuckDB execution. DuckLake support is reached through DuckDB SQL and the DuckLake extension. If extension availability diverges from the DuckDB shell, surface that through `doctor` rather than adding a second default runtime.

## Local Execution

`query` is the AI-agent workhorse. It should support inline SQL and SQL files, execute against local DuckDB or DuckLake, cap JSON output with `--max-rows`, and write a run manifest. Run manifests should contain:

- target engine and database
- statement source
- statement SHA-256 fingerprint
- statement byte length
- rows returned
- timestamp

Full SQL should be opt-in through `--record-sql` so agents can preserve reproducibility when wanted without accidentally serializing credentials or sensitive literals.

The agent-facing surface should stay small: `doctor`, `skills install`, `init`, `explain`, `pull`, `ls`, `describe`, `show`, `query`, and `inspect`. `plan` and `route` remain compatibility aliases for `explain`; `run` remains a compatibility alias for `query`; `cat` remains a compatibility alias for `show`; `status` remains a compatibility alias for `inspect`.

Argument names should preserve the product boundary:

- `--local-target` selects the local DuckDB or DuckLake target for `init`, `explain`, and `pull`.
- `--engine` selects the SQL execution engine for `query`.
- `--mode local|remote` makes the execution boundary explicit.
- `--remote quack://host:port` is valid only with `query --engine duckdb --mode remote`.

## Inspection And Dry Run

Inspection commands should stay read only and bounded:

- `ls`: list visible tables.
- `describe`: return a table schema.
- `show`: return a limited sample.
- `query --dry-run`: run non-executing `EXPLAIN`, skip manifests, reject statement sequences and `EXPLAIN ANALYZE`, and classify as a read operation.

Normal `query` writes a manifest, so local normal `query` is a local write even when SQL is read only. This keeps the permission model based on actual side effects instead of SQL text alone.

## DuckLake Snapshots

DuckLake supports snapshots and time travel through DuckDB functions such as `snapshots()`, `current_snapshot()`, `last_committed_snapshot()`, and SQL `AT (VERSION => ...)` or `AT (TIMESTAMP => ...)`. Altaika should later expose these as agent commands:

```bash
altaika snapshots list --engine ducklake
altaika time-travel query --version 3 "SELECT count(*) FROM events"
```

Until those commands exist, prefer documenting the SQL path through `query` and keep snapshot metadata in manifests when it can be read cheaply.

## Quack

Quack should not be the default local path. It is part of the default OSS feature set because remote DuckDB is a natural DuckDB-family workflow, but runtime use remains explicit.

Use Quack for:

- remote DuckDB sessions
- shared team DuckDB endpoints
- probes where the source or staging service is already DuckDB-shaped

Do not use Quack as:

- the default local workspace engine
- a replacement for BigQuery extraction
- a generic warehouse abstraction layer

## BigQuery And Other Connectors

BigQuery, Snowflake, Databricks, PostgreSQL, and Altertable should be implemented as explicit `pull` sources. The OSS contract stays generic:

```text
remote source -> bounded local copy -> manifest
```

BigQuery uses the optional ADBC path. When `bigquery-adbc` is enabled, Altaika executes bounded SQL through the BigQuery ADBC driver, streams Arrow batches into the local DuckDB or DuckLake target through `duckdb-rs`, and writes the normal pull manifest. Missing driver, billing project, or Google credentials must return structured `connector_setup` JSON.

After `pull`, local `query` steps can transform, validate, and prepare data in DuckDB or DuckLake. Altertable can add value later through optional publish workflows, richer source profiles, lineage, validation, and semantic metadata.

## Auth

Local DuckDB and DuckLake do not need an Altaika account. Remote pull sources should use runtime credentials from environment variables, native cloud identity, or explicit profiles that do not serialize secrets. Manifests should record source identity and query shape, not tokens.

## DuckDB, DuckLake, And Quack Notes

- DuckDB is the local execution engine and CLI integration point.
- DuckLake is the durable local catalog target.
- Quack is explicit remote DuckDB transport.
- DataFusion and source-specific readers are optional fetch layers into DuckDB or DuckLake, not default local engines.
