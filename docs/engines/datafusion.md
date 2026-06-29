# DataFusion Engine

Status: v0 engine.

DataFusion is the local OSS engine for examples, conformance, tests, and agent sandboxes that do not need Altertable credentials.

Supported local sources in v0:

- CSV through `--csv schema.table=path`
- Parquet through `--parquet schema.table=path`

DataFusion fills `SourceProfile` from registered table providers and Arrow schemas. It should remain the default local kernel until a DuckDB or DuckLake scenario needs behavior DataFusion does not cover.

DataFusion has no authentication in v0. It should not invent remote credential handling.

Use this engine to validate the typed planning path. Tests should build or execute typed operations instead of relying only on hand-written SQL strings.
