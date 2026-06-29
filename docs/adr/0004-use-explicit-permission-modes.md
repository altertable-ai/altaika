# 0004. Use Explicit Permission Modes

## Status

Accepted

## Context

AI agents may run commands that read data, write local state, cross remote boundaries, or mutate remote systems. The CLI needs a simple policy that agents can reason about without interactive prompts.

`auto` can be misread as automatic routing. In Altaika, automatic hidden local/remote switching is not acceptable because it can hide cost, latency, credential use, or data movement.

## Decision

Altaika has three invocation-level permission modes:

- `permission`: default. Allows explain, local read inspection, doctor, inspect, and dry-run query.
- `auto`: allows local writes.
- `allow`: allows remote reads and writes for that invocation.

`auto` is approval for local side effects. It is not automatic local or remote route switching.

## Consequences

Blocked commands return `approval_required` JSON. Agents must inspect `stats.permission` and rerun with the narrowest acceptable mode. Remote Quack and source platform reads require `--permission allow`. Normal local `query` writes a manifest, so it requires `--permission auto`.
