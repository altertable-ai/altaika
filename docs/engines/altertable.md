# Altertable Engine

Status: v0 engine.

Altertable is the product-native engine. Execution uses Flight SQL. Optional app context can enrich planning through internal tool endpoints when `ALTAIKA_ALTERTABLE_APP_URL` and `ALTAIKA_ALTERTABLE_ENVIRONMENT_ID` are set.

Runtime credentials come from environment variables:

- `ALTERTABLE_USER`
- `ALTERTABLE_PASSWORD`
- `ALTAIKA_ALTERTABLE_FLIGHT_HOST`
- `ALTAIKA_ALTERTABLE_INSECURE`

Profiles must not serialize secrets. Future profile fields should cover host, organization, environment, catalog, schema, and auth method.

Altertable should be better than a generic SQL engine because it can return richer `SourceProfile` data from cached schema, semantic context, lineage, data quality hints, and app-level knowledge.

Local copies should start as explicit bounded exports from Flight SQL Arrow streams to Parquet, DuckDB, or DuckLake. They must write a manifest with source, query, schema, snapshot id when available, generated time, row count, and local data path.
