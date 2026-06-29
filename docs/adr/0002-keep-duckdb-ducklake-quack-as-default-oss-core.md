# 0002. Keep DuckDB, DuckLake, And Quack As The Default OSS Core

## Status

Accepted

## Context

The project needs to be useful as OSS without requiring warehouse credentials, Altertable infrastructure, BigQuery drivers, or heavyweight connector stacks. It should still support future source pulls from BigQuery, Snowflake, Databricks, PostgreSQL, object stores, and Altertable.

DuckDB and DuckLake provide the local workspace runtime and durable local state. Quack provides explicit remote DuckDB transport. DataFusion, ADBC, and platform integrations are valuable when they improve remote fetch, Arrow streaming, projection, filtering, or pushdown, but they should not be mandatory for local agent work.

## Decision

The default feature set is DuckDB, DuckLake, and Quack.

DataFusion, ADBC, BigQuery, and Altertable platform integration remain opt-in features.

## Consequences

Default installs stay local-first and agent-friendly. Remote systems are crossed only through explicit `pull` or explicit Quack commands. Local CSV, Parquet, DuckDB, and DuckLake operations should use DuckDB directly unless measured evidence shows another engine is better. BigQuery support must report missing driver/auth/setup as structured JSON instead of silently falling back.
