# CLAUDE.md

Follow [AGENTS.md](AGENTS.md) for repository instructions.

Altaika is an AI CLI tool for DuckDB, DuckLake, Quack, and optional source connectors. Optimize changes for machine-readable command behavior, reproducible local state, and explicit remote boundaries.

Before claiming a change is ready, run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

For WASI compatibility checks, run:

```bash
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --no-default-features --features wasi
```
