# Security Policy

Altaika is designed for AI agents that may touch local data and remote warehouses. Treat every connector, manifest, and command output as potentially sensitive.

## Reporting A Vulnerability

Use GitHub private vulnerability reporting when it is available for this repository. If it is not available, open a minimal public issue that asks for a private security contact and do not include exploit details, credentials, dataset names, customer names, or private infrastructure paths.

## Handling Secrets

- Do not store credentials in manifests, examples, tests, or config files.
- Prefer environment variables, platform-native credential providers, or explicit profile names that do not serialize secret values.
- Keep remote Quack tokens out of command output. CLI arguments that may contain tokens should stay hidden from help output where possible.
- Use `--permission allow` only for an invocation where remote access has been approved.

## Security-Relevant Checks

Before changing connector, permission, Quack, or manifest behavior, run:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

Add focused tests for approval boundaries, redacted output, and manifest contents when changing those areas.
