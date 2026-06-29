# Contributing An Engine

Engines implement `altaika_engine::Engine` and keep the same small command contract across platforms: `ls`, `describe`, `cat`, and explicit `sql`.

Required behavior:

- `name` returns the stable engine name used by the CLI.
- `dialect` returns the SQL dialect the engine executes.
- `capabilities` reports metadata and limit pushdown support.
- `inspect` returns a `SourceProfile` without copying row data.
- `execute` runs `PlannedSql.rendered_sql` and returns Arrow record batches.

Do not add a crate until it executes at least one v0 command end to end. Planned engines should start as docs so contributors understand the shape without carrying empty code.

Remote transports can be documented or exposed as beta SQL paths before they become engines. Quack is one example: it transports DuckDB SQL, so it belongs under the DuckDB engine lane unless runtime evidence proves it needs a separate crate.

`SourceProfile` is the main product boundary. Generic engines can fill it from `information_schema` or provider schemas. Altertable can enrich it with cached schema, semantic context, lineage, and quality hints.

Authentication stays engine-owned. Altaika profiles may store non-secret context such as host, organization, environment, catalog, schema, warehouse, role, and auth method. Secrets must come from environment variables, OS-native secure storage, or the upstream platform's own profile system.

Local copies and mirrors must be explicit, bounded, read-only by default, and accompanied by a manifest. `describe` must never copy rows.

Every engine should pass the conformance cases under `tests/conformance/cases`. These cases check behavior, not exact SQL text.
