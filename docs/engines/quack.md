# Quack Remote Transport

Status: beta support through `--engine duckdb-beta`.

Quack is DuckDB's remote protocol. It is not a SQL dialect and should not be exposed as a target dialect in Altaika. Treat it as a possible transport for the DuckDB engine, next to local DuckDB files, DuckLake attaches, and remote object storage reads.

Current evidence:

- DuckDB docs say Quack turns a DuckDB instance into a server that clients can connect to over HTTP.
- DuckDB docs mark Quack as beta and under active development, available in DuckDB v1.5.3.
- DuckDB docs say the server exposes the full SQL surface of its DuckDB session.
- DuckDB 1.5.4 installed through `https://install.duckdb.org` exposes the `quack` extension in `duckdb_extensions()`.
- `quack_protocol` 0.1.0 is a Rust client SDK for DuckDB's experimental Quack remote protocol.
- `quack_protocol` returns Quack rows and JSON helpers, not Arrow record batches.

Beta CLI path:

```bash
ALTAIKA_DUCKDB_BIN=$HOME/.duckdb/cli/latest/duckdb altaika --engine duckdb-beta auth
ALTAIKA_DUCKDB_BIN=$HOME/.duckdb/cli/latest/duckdb altaika --engine duckdb-beta sql "SELECT 1 AS one"
```

This path shells out to DuckDB's own CLI with `-json`. It supports explicit SQL and auth diagnostics only. It does not implement `ls`, `describe`, `cat`, or snapshots.

Use Quack for:

- Remote DuckDB to DuckDB data exchange when the server is already running.
- Bounded migration probes from a remote DuckDB session into a local Parquet, DuckDB, or DuckLake mirror.
- Append or query workflows where the remote endpoint is explicitly configured and authenticated.

Performance stance:

- DuckDB docs describe Quack as one request and response per query after connection setup, with large results fetched in chunks.
- Altaika should still benchmark Quack against direct DuckDB, DuckLake attach, and Parquet export before making it the default exchange path.
- Use `EXPLAIN ANALYZE`, row count, bytes transferred, wall time, and output manifest size as the first comparison fields.

Do not use Quack for:

- The default local working-copy engine. Use DuckDB or DuckLake directly.
- The public Altertable execution path. Altertable remains Flight SQL plus app context.
- Generic Snowflake, Databricks, or PostgreSQL migration. Those engines should start with their native APIs and only export bounded mirrors later.
- A new `Dialect` variant. Quack transports DuckDB SQL.

Eligibility for full runtime integration:

- Local DuckDB can install and load the Quack extension.
- A live Quack server is available through `QUACK_SERVER_URI`.
- Authentication comes from `QUACK_AUTH_TOKEN` or an upstream secret provider. Altaika must not store the token in profile files.
- Result conversion to Altaika's Arrow `RecordStream` is implemented and tested.
- Authorization policy is explicit enough for non-local servers. Quack's default authorization is permissive, so production use needs a server-side authorization hook or a trusted boundary.

Minimal full-engine shape:

```text
altaika-engine-duckdb
  local file or DuckLake attach -> DuckDB connection
  remote Quack URI             -> quack_protocol client
```

This keeps Quack decoupled from the existing DataFusion and Altertable engines while preserving the same typed CLI contract.
