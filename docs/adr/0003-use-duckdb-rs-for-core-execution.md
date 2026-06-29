# 0003. Use duckdb-rs For Core Execution

## Status

Accepted

## Context

Shelling out to the DuckDB CLI makes execution depend on whichever DuckDB binary happens to be installed. During local testing, the system DuckDB CLI exposed an older Quack function surface than the embedded DuckDB runtime. That made Quack server testing fail even though the embedded runtime supported the needed functions.

AI agents need stable JSON, structured errors, explicit permissions, and predictable feature checks.

## Decision

Altaika uses `duckdb-rs` for core DuckDB execution.

The external DuckDB CLI may still be used manually to host a production-like backend DuckDB process, but Altaika must not depend on shelling out for core query, inspect, DuckLake, or Quack client behavior.

## Consequences

Runtime behavior is tied to the bundled DuckDB version from `duckdb-rs`. `doctor` must expose embedded DuckDB version, extension visibility, and Quack function readiness. Quack server support can be tested through the embedded runtime even when the system DuckDB CLI is older. WASI/no-default builds must return structured unsupported-runtime errors for DuckDB-dependent commands.
