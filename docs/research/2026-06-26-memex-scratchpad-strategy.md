# MemEx Scratchpad Strategy

Date: 2026-06-26

## Decision

Adopt the MemEx shape for agent data work, but do not make the typed CLI the primary agent interface.

The best Altertable strategy is:

1. A persistent programmable scratchpad for agents.
2. Existing MCP tools injected as typed functions.
3. DuckDB SQL and Altertable `query_lakehouse` as the main data action path.
4. Bounded local snapshots into Parquet, DuckDB, or DuckLake for repeated local loops.
5. Rust for engines and CLI plumbing, not as the language agents write during a rollout.

## Why

Databricks MemEx says the core win is a typed Python kernel that keeps tool outputs as objects, lets the agent transform them with code, and only returns selected print output to the model context. Their reported enterprise structured retrieval results show frontier models gaining 2 to 5 accuracy points at 25 to 30 percent lower token cost, and open weight models nearly doubling accuracy at 40 to 50 percent lower token cost.

Anthropic Programmatic Tool Calling makes the same point: code orchestration avoids one model round trip per tool call, processes intermediate results in the code execution environment, and returns only final output to the model. Cloudflare Code Mode makes the MCP specific version: convert MCP tools into a TypeScript API, run code in a sandbox, and route API calls back through MCP authorization.

Altertable already has the pieces:

1. `initialize`, `list_catalogs`, `get_catalog`, `query_lakehouse`, `explain_sql`, and `validate_sql` are real MCP tools. The production MCP session listed 10 catalogs on 2026-06-26.
2. `query_lakehouse` caps each statement payload to 50,000 estimated tokens, paginates with `offset`, and enriches results with query log duration and stats. See `/Users/florianvaleye/Documents/workspace/backend/app/app/mcp/tools/query_lakehouse.rb`.
3. `get_catalog` renders catalog profiles with schemas, tables, columns, semantic endorsement, measures, and relations. See `/Users/florianvaleye/Documents/workspace/backend/app/app/mcp/tools/get_catalog.rb`.
4. The super agent workflow already tells agents to discover schema first, then use DuckDB SQL. See `/Users/florianvaleye/Documents/workspace/backend/app/app/workflows/super_agent.yml`.
5. The Rust API already streams Arrow through Flight SQL. See `/Users/florianvaleye/Documents/workspace/backend/api/src/common/worker_client/mod.rs`.
6. DuckLake attach is already present in the worker database definition. See `/Users/florianvaleye/Documents/workspace/backend/api/src/common/ducklake_attach.rs`.

External sources:

1. Databricks MemEx blog: `https://www.databricks.com/blog/memex-programmable-scratchpad-llm-agents`.
2. Anthropic advanced tool use: `https://www.anthropic.com/engineering/advanced-tool-use`.
3. Cloudflare Code Mode: `https://blog.cloudflare.com/code-mode/`.
4. MCP tools concept: `https://modelcontextprotocol.io/docs/concepts/tools`.
5. DuckLake DuckDB introduction: `https://ducklake.select/docs/stable/duckdb/introduction`.
6. DuckDB CLI docs: `https://duckdb.org/docs/current/clients/cli/overview.html`.
7. DuckDB Python docs: `https://duckdb.org/docs/current/clients/python/overview.html`.

## Local Benchmark

Dataset: 50,000 row CSV generated at `/tmp/altaika_events.csv`.

Query:

```sql
SELECT id
FROM read_csv_auto('/tmp/altaika_events.csv')
WHERE event_type = 'purchase'
LIMIT 10
```

Command:

```bash
hyperfine --warmup 3 --runs 20 \
  "duckdb -json -c \"SELECT id FROM read_csv_auto('/tmp/altaika_events.csv') WHERE event_type = 'purchase' LIMIT 10\"" \
  "python3 /tmp/altaika_bench_python.py" \
  "node /tmp/altaika_bench_node.mjs" \
  "bun /tmp/altaika_bench_bun.js" \
  "target/debug/altaika --csv public.events=/tmp/altaika_events.csv cat local/events --columns id --filter event_type:=purchase --limit 10"
```

Results:

| path | mean |
|---|---:|
| DuckDB CLI direct | 28.5 ms |
| Bun wrapper around DuckDB CLI | 37.3 ms |
| Python wrapper around DuckDB CLI | 51.4 ms |
| Node wrapper around DuckDB CLI | 72.2 ms |
| Rust/DataFusion debug CLI | 93.0 ms |

The result is not a native binding benchmark. Native Python and Node DuckDB bindings were not installed locally. It does prove the thing that matters for this branch: spawning a CLI and rebuilding context per action is slower than direct DuckDB, and the current Rust/DataFusion CLI is not the fastest scratchpad execution path.

A release build with the workspace LTO profile was interrupted after several minutes at link time. That is a good reason not to put the fast iteration scratchpad layer in Rust. Rust still remains the right place for the lakehouse worker and stable CLI engine code.

The earlier headless agent benchmark in `/Users/florianvaleye/Documents/workspace/altaika/bench/report.md` also argues against typed only:

| arm | success | median turns | median tokens | median cost |
|---|---:|---:|---:|---:|
| Typed CLI | 94% | 2 | 513 | $0.0746 |
| Altaika SQL wrapper | 100% | 2 | 273 | $0.0611 |
| Raw SQL | 94% | 2 | 246 | $0.0602 |

## Runtime Choice

Use this decision table:

| Need | Best default | Why |
|---|---|---|
| Agent data analysis, local objects, dataframe transforms | Python scratchpad | Matches MemEx, best model prior, best DuckDB and data ecosystem |
| MCP tool orchestration in a web or isolate sandbox | TypeScript scratchpad | Matches Cloudflare Code Mode and JSON Schema to type generation |
| Lakehouse execution, Flight SQL, DuckLake attach, CLI internals | Rust | Existing stack, Arrow and DataFusion integration, strong compiled boundary |
| Fast one off local SQL | DuckDB CLI | Measured fastest available local path |

Default for Altertable: Python scratchpad first, backed by Altertable MCP tools and Rust lakehouse execution. Add a TypeScript backend only when the host sandbox is Workers, Bun, or an MCP client that wants generated TS APIs. Do not make agents write Rust.

## Architecture

```text
LLM
  -> scratchpad kernel
       -> list_catalogs()
       -> get_catalog()
       -> query_lakehouse()
       -> explain_sql()
       -> validate_sql()
       -> snapshot()
       -> submit()
  -> selected stdout and submit payload only
```

The scratchpad keeps:

1. Catalog profiles.
2. Query results as native objects.
3. Arrow batches or compact row objects.
4. Local snapshot manifests.
5. Helper functions written during the rollout.

The model should see summaries, printed tables, and final `submit()` payloads. It should not see every raw tool result by default.

## DuckDB And DuckLake

Use DuckDB and DuckLake as the local working copy substrate, not as a replacement for governed live execution.

Modes:

1. Metadata only. Use `get_catalog`, cached schema, semantic hints, partition and sort metadata.
2. Live SQL. Use `query_lakehouse` for fresh results and governed execution.
3. Explicit bounded local copy. Use `snapshot` with columns, filters, limits, samples, or a named snapshot, then query locally with DuckDB or DuckLake.

DuckLake becomes important once repeated local loops need snapshot identity, time travel, read only attach, or shared object storage. Do not auto download a full remote table from `describe`.

## Criteria

Choose an implementation only if it improves at least one of these without weakening authorization:

1. Fewer model round trips for multi tool workflows.
2. Lower token materialization of intermediate results.
3. Faster repeated analysis over the same subset.
4. Stronger schema and semantic grounding through catalog profiles.
5. Clear sandbox controls: filesystem, network egress, resource caps, and secret isolation.
6. Reproducible local artifacts: snapshot manifests with source, query, schema, row count, and generated time.

## Current Branch Scope

This branch keeps the OSS CLI as a useful substrate and adds plan coverage for `ls` and `describe`. The strategic correction is that `altaika` should support scratchpad agents rather than replace their code action space with more typed verbs.

Skipped for this branch:

1. A production sandbox service.
2. Native Python DuckDB packaging.
3. Native TypeScript DuckDB packaging.
4. A DuckLake engine crate.
5. Any background sync daemon.
