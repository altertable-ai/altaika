# PostgreSQL Engine

Status: planned after v0.

Start with live read-only execution and metadata from:

- `information_schema.schemata`
- `information_schema.tables`
- `information_schema.columns`

Required profile fields:

- host
- database
- schema
- role or user context
- auth method

Use PostgreSQL-native authentication conventions first. Altaika should not store database passwords in profile files.

Local copy should come later as an explicit bounded export, for example a query stream or `COPY` result into Parquet, DuckDB, or DuckLake.
