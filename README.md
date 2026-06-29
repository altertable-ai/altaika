# altaika

`altaika` is an OSS Rust CLI for AI agents working with data platforms.

V0 supports:

- `altaika ls`
- `altaika completions`
- `altaika describe`
- `altaika cat`
- `altaika plan`
- `altaika snapshot`
- `altaika sql`
- `altaika auth`
- `altaika agent schema`
- `altaika skills list`

The first engines are DataFusion for local OSS use and Altertable for product-native execution.

Command help includes an embedded markdown skill for agents:

```bash
altaika cat --help
```

`altaika agent schema` also returns each command's `help` command and source `skill` markdown path.
Use `altaika --engine altertable auth --check` for a live credential smoke, and
`altaika agent issue-template` to prepare a GitHub issue without exposing secrets.
Use `altaika skills list` for a compact local index of embedded command skills.
Use `altaika completions zsh` to print shell completion scripts.

DuckDB beta features are available through the DuckDB CLI binary:

```bash
ALTAIKA_DUCKDB_BIN=$HOME/.duckdb/cli/latest/duckdb altaika --engine duckdb-beta auth
ALTAIKA_DUCKDB_BIN=$HOME/.duckdb/cli/latest/duckdb altaika --engine duckdb-beta sql "SELECT 1 AS one"
```

Local DataFusion sources are explicit:

```bash
altaika --csv public.events=events.csv ls local/public
altaika --csv public.events=events.csv ls --long local/public
altaika --csv public.events=events.csv describe local/public/events
altaika --csv public.events=events.csv cat local/events --columns id --limit 10
altaika --csv public.events=events.csv cat local/events --filter id:>1 --limit 10
altaika plan ls local/public --target snowflake
altaika plan describe local/public/events --target postgresql
altaika plan cat local/events --target ducklake --columns id --limit 10
altaika plan cat local/events --target snowflake --columns id,event_name --limit 10
altaika --format ndjson --csv public.events=events.csv cat local/events
altaika --csv public.events=events.csv snapshot local/events --out events.parquet --columns id --limit 100
altaika sql "SELECT 1 AS one"
altaika completions zsh
altaika --engine altertable auth
altaika agent schema
altaika agent schema --compact
altaika skills list
```

Development checks:

```bash
cargo fmt
cargo clippy -p altaika-sql -p altaika-cli --all-targets -- -D warnings
cargo test -p altaika-sql -p altaika-cli
cargo deny check
```

Altertable uses runtime credentials only:

```bash
ALTERTABLE_USER=agent@example.com ALTERTABLE_PASSWORD=... altaika --engine altertable sql "SELECT 1 AS one"
```

Engine contribution docs live under `docs/engines/`, including the Quack beta transport lane. Conformance cases live under `tests/conformance/cases/`.
