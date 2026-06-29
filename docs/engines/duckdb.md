# DuckDB And DuckLake Engine

Status: planned after the first v0 CLI path works.

Use this future engine for local working copies, direct DuckDB files, direct DuckLake files, and remote file access through DuckDB extensions.

Start with the DuckDB extension path for DuckLake. Do not build a custom Rust DuckLake client before a required scenario proves that the DuckDB extension path is insufficient.

Initial scope:

- Attach a local DuckDB or DuckLake file.
- Run `describe`.
- Run bounded `cat`.
- Never auto-download a full remote table.

Quack is available through the beta `duckdb-beta` SQL path today, not as a separate dialect. See `docs/engines/quack.md`. Keep it there until a full DuckDB engine can return Arrow `RecordStream` batches for the typed commands.

DuckDB and DuckLake are the local working-copy substrate, not the entire Altaika product. Altaika should still preserve native platform execution for freshness, permission semantics, billing controls, and pushdown.

Do not claim DuckDB, DuckLake, or MotherDuck lack AI tooling. The useful claim is narrower: Altaika provides one terminal-first command contract for agents across local files, DuckLake, DataFusion, Altertable, and future warehouse engines.
