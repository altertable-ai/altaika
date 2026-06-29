# 0001. Use The CLI As The Primary Agent Surface

## Status

Accepted

## Context

Altaika targets AI agents that need discoverable commands, structured JSON output, explicit permissions, manifests, and skills. Agents may run from Codex, Claude, CI, local shells, backend integration tests, or future MCP surfaces.

DuckDB extensions are useful when SQL-native table functions, macros, planner integration, or zero-copy execution are required. They are less convenient as the first product surface for command discovery, skill installation, auth boundaries, local workspace setup, and cross-platform agent usage.

## Decision

Altaika is a CLI first. DuckDB extension work is deferred until a SQL-native bottleneck appears.

The CLI owns command discovery, JSON response contracts, permission boundaries, skill installation, local workspace lifecycle, Quack orchestration, and manifests.

## Consequences

Agents can call Altaika from any environment that can run a process. The OSS core stays independent from one DuckDB process. DuckDB extension work remains possible later for table functions or SQL-native workflows. Every command must stay machine-readable and predictable.
