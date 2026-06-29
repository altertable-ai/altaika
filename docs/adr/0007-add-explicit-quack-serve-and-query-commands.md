# 0007. Add Explicit Quack Serve And Query Commands

## Status

Accepted

## Context

Altaika needs to test and document the production-like shape where a backend-owned DuckDB process exposes a DuckDB workspace through Quack and the CLI connects as an external AI client.

Local testing showed that the system DuckDB CLI can expose an older Quack function surface than the embedded `duckdb-rs` runtime. The installed DuckDB CLI exposed `quack()` but not `quack_serve`, while the embedded runtime exposed `quack_serve`, `quack_query`, and `quack_stop`.

Quack server readiness can also include sensitive values, such as an `auth_token`, in returned rows. AI-agent output must not echo those secrets.

## Decision

Altaika adds explicit Quack commands:

- `altaika --permission allow quack serve`
- `altaika --permission allow quack query`

`quack serve` uses the embedded `duckdb-rs` runtime, prints one JSON readiness envelope, redacts sensitive readiness fields, then keeps the foreground process alive. `quack query` is a terminal-like remote SQL entrypoint that does not write local manifests.

Existing `query --mode remote`, `ls --mode remote`, `describe --mode remote`, and `show --mode remote` remain supported.

## Consequences

- Agents can start a local Quack server for tests without relying on the system DuckDB CLI.
- Backend production can use the same command contract while placing the process behind supervision, TLS termination, and token management.
- Remote Quack serving and querying require `--permission allow`.
- `doctor` must report Quack function readiness, not just extension installation.
- Quack readiness output must redact token-like fields.
