# AI CLI Best Practices

Altaika is an AI CLI tool for DuckDB, DuckLake, and Quack. It should optimize for agents that need reliable local execution, bounded remote access, compact outputs, explicit permissions, and reproducible state.

## Command Surface

Keep the agent-facing surface small and predictable:

- `doctor`: inspect local runtime readiness, and optionally a remote Quack endpoint.
- `skills install`: install the Altaika agent skill, with `--dry-run` available.
- `explain`: decide `local` or `remote` before work happens. `plan` and `route` are compatibility aliases.
- `pull`: cross a source boundary explicitly, then write local state.
- `ls`, `describe`, `show`: inspect a local or explicit remote DuckDB catalog without writing manifests. `cat` is a compatibility alias.
- `query --dry-run`: run `EXPLAIN` without writing manifests.
- `query`: execute SQL and write a run manifest. `run` is a compatibility alias.
- `quack serve`: start a foreground Quack server for backend-like local testing.
- `quack query`: run terminal-like remote DuckDB SQL without writing a local manifest.
- `inspect`: summarize local workspace state, manifests, and prior runs. `status` is a compatibility alias.

Avoid forcing agents to choose between many verbs when the real decision is only local or remote. Add new commands only when they clarify permissions, output size, or operation intent.

## Response Contract

Every command must return JSON with the same top-level fields:

```json
{
  "kind": "query",
  "schema_version": "1.0",
  "skill": "local_query",
  "mode": "local",
  "stats": {},
  "data": {}
}
```

Rules:

- `skill` names the capability used by the command.
- `mode` is either `local` or `remote`.
- `stats` contains small information an agent needs for planning.
- `stats.operation` names the operation and class.
- `stats.permission` reports the permission mode, approval decision, and reason.
- `stats.rows_truncated` reports when query output hit the agent row cap.
- `data` contains detailed command output.
- stdout is machine-readable JSON. stderr is for structured errors.
- Errors must include `kind`, `schema_version`, `skill`, `mode`, `stats`, and `error`.

## Skill Names

Use stable skill names. They are part of the CLI contract.

- `environment_doctor`
- `skills_install`
- `routing_explain`
- `source_pull`
- `catalog_list`
- `table_describe`
- `table_preview`
- `query_explain`
- `local_query`
- `remote_query`
- `workspace_status`
- `workspace_init`
- `approval_required`
- `connector_setup`
- `error_report`

Do not encode provider names in core skills. A future Altertable connector should still report `source_pull` or `publish_target`, not make the OSS core depend on Altertable naming.

## Permissions

Default to `--permission permission`.

| Operation class | `permission` | `auto` | `allow` |
| --- | --- | --- | --- |
| `explain` | allow | allow | allow |
| `local_read` | allow | allow | allow |
| `local_write` | approval required | allow | allow |
| `remote_read` | approval required | approval required | allow |
| `remote_write` | approval required | approval required | allow |

`allow` is an explicit approval for one invocation. It does not bypass secret redaction, downstream authorization, or command validation.

Examples:

- `doctor`, `inspect`, `ls`, `describe`, `show`, and `query --dry-run` are allowed in default mode when they are local read operations.
- `init`, `pull`, `skills install`, and normal local `query` require `--permission auto`.
- Remote Quack inspection and remote `query` require `--permission allow`.

## Stats

Return stats on every command. Keep them small, numeric, and easy to compare.

Recommended stats:

- `rows_returned`
- `rows_truncated`
- `row_limit`
- `row_count`
- `estimated_rows`
- `estimated_data_size_bytes`
- `bytes_read`
- `bytes_written`
- `latency_ms`
- `duration_ms`
- `statement_bytes`
- `statement_sha256`
- `manifest_written`
- `cache_hit`
- `remote_calls`
- `missing_signal_count`
- `operation`
- `permission`

For `explain`, stats help an agent choose local or remote. For `pull`, stats summarize the data that crossed a boundary. For inspection commands, stats summarize read-only catalog or row preview work. For `query`, stats summarize SQL execution and manifest behavior. For `inspect`, stats summarize workspace inventory.

## Token Budget

Assume command output is inserted into an LLM context.

- Default to compact JSON.
- Return row counts and schema summaries before returning rows.
- Require `--limit` for previews and keep low defaults.
- Cap query rows with `--max-rows`, default to bounded output, and ask before raising caps on large or remote results.
- Prefer pointers to manifests over repeating full metadata.
- Record SQL fingerprints by default, not full SQL.
- Make full SQL recording explicit with `--record-sql`.
- Add `--columns`, `--filter`, `--limit`, and later `--max-bytes` for every source pull.
- Add `--summary` and `--jsonl` later for large results.

Stable prompts and stable tool schemas improve cacheability in model APIs that support prompt caching. Keep command help, JSON keys, and agent instructions deterministic.

## Local First

Default to local work.

- Local CSV and Parquet should use DuckDB scanners.
- Local DuckDB and DuckLake table sources should use read-only attach plus bounded copy.
- Local workspace state should live under `.altaika/`.
- DuckLake should be the durable local catalog when schema history or multi-step local work matters.
- DuckDB should stay available for lightweight scratch work.
- Remote source platform reads happen only through `pull`.
- Remote DuckDB execution happens only through explicit Quack commands.
- `query --dry-run` must reject statement sequences and `EXPLAIN ANALYZE`; both can execute side effects.

Local execution gives agents a low latency sandbox and prevents repeated warehouse calls.

## Remote Boundaries

Remote mode must be explicit.

- `explain` can recommend remote work, but it must not perform remote work.
- `pull` is the source platform read boundary in the core CLI.
- `query --mode remote` is allowed only for explicit remote DuckDB execution through Quack.
- Remote connectors must report missing signals before running costly work.
- Remote connectors must never serialize credentials into manifests.
- Plan decisions should include data size, rows, latency, confidence, missing signals, and recommendation.

DataFusion belongs in the remote source fetch layer when it helps connector behavior, Arrow streaming, projection, filtering, or pushdown. Do not put DataFusion on top of DuckDB by default. Quack is an explicit remote DuckDB transport. Altertable is an optional source connector or publish target, not part of the OSS core. Migration can be achieved by composing the CLI primitives, but it is not a top-level CLI feature.

Connector setup failures should still be useful. BigQuery setup errors include `stats.auth`, `stats.driver`, generated `stats.source_sql`, `stats.phase_error` when available, and `stats.migration_goal.parity_checks`. Agents should use these fields to decide whether the missing piece is build feature, driver installation, credentials, billing project, or runtime load.

## Performance

Use the cheapest engine for the route.

- For local Parquet and CSV, prefer DuckDB scanners and push projection or filter work into SQL.
- For remote platforms, push projection and filters into the source before localizing data.
- For remote DuckDB, use Quack only when the endpoint is intentionally selected.
- Write manifests once and reuse local state through `inspect`.
- Avoid repeatedly spawning remote reads for agent iteration.
- Avoid returning huge result sets. Return stats, hashes, schema, manifests, and samples.
- Stop collecting query rows once the agent row cap is reached and report truncation.
- Use embedded DuckDB through `duckdb-rs` for deterministic local execution.
- Use read-only DuckDB connection options for inspection paths when possible.
- Measure build time and startup time before adding more default native features.

Do not add complexity for unmeasured performance. Add measurements to `stats` first.

## Auth And Secrets

Keep auth outside manifests.

- Use environment variables, native cloud identity, or explicit local profiles.
- Do not write tokens, DSNs with passwords, or cloud keys to manifests.
- Default to SQL fingerprints, not full SQL.
- Make remote actions visibly remote through `mode: "remote"`.
- Prefer short-lived credentials for remote connectors.

## Failure Design

Failures must help the next agent action.

Error JSON should include:

- `skill`
- `mode`
- `stats`
- `error`
- later: `error_code`, `retryable`, `next_action`

Avoid prose-only errors. An agent should be able to branch on machine-readable fields.

## Useful Options

Current or planned options:

- `--permission permission|auto|allow`
- `--mode local|remote`
- `--local-target ducklake|duckdb`
- `--source-latency-ms`
- `--limit`
- `--max-rows`
- `--max-bytes`
- `--columns`
- `--filter`
- `--timeout-ms`
- `--sample`
- `--summary`
- `--jsonl`
- `--record-sql`
- `--manifest-out`
- `--quack-token`
- `--quack-disable-ssl`
- `--dry-run`

Do not add every option at once. Add them when the corresponding command has a real implementation and tests.

## DuckDB, DuckLake, And Quack Notes

- DuckDB CLI runs should stay deterministic with `-init /dev/null`.
- DuckDB import and scan paths should stay bulk oriented.
- DuckLake is the durable local catalog path.
- Quack is explicit remote DuckDB transport, not the local runtime.
- Quack production use should sit behind TLS termination.
