# 0008. Use ADBC Arrow For Optional BigQuery Pulls

## Status

Accepted

## Context

Altaika needs BigQuery support for bounded source pulls without making BigQuery, ADBC, Arrow append APIs, or Google credentials part of the default OSS install.

ADBC returns Arrow record batches from SQL execution. DuckDB's Rust crate can append Arrow record batches when its Arrow appender feature is enabled. That gives Altaika a direct source-to-local path without adding DataFusion to the default runtime.

## Decision

`bigquery-adbc` enables the ADBC driver manager and DuckDB's Arrow appender feature.

BigQuery `pull` builds bounded BigQuery SQL, configures the BigQuery ADBC driver, requires a billing project, executes the query, streams Arrow batches into the selected local DuckDB or DuckLake target, and writes the normal pull manifest.

The default OSS feature set remains DuckDB, DuckLake, and Quack. BigQuery remains optional.

## Consequences

- Agents get a real local BigQuery-to-DuckLake or BigQuery-to-DuckDB path when credentials and the driver are available.
- Missing driver, billing project, or Google credentials return structured `connector_setup` JSON instead of partial local state.
- DataFusion is not required for BigQuery MVP ingestion. It can still be introduced for connectors where it improves projection, filtering, object-store reads, federation, or execution planning.
- The BigQuery feature pulls extra native and Arrow surface only when explicitly enabled.
