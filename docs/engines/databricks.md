# Databricks Engine

Status: planned after v0.

Start with the Statement Execution API and Unity Catalog style names.

Required profile fields:

- host
- catalog
- schema
- warehouse id
- disposition
- auth method

Reuse Databricks native environment and profile conventions. Do not add Databricks credential storage to Altaika profiles.

The first implementation should support read-only typed discovery and bounded reads. Local copy should come later through explicit bounded export or Delta-backed local reads when permissions allow it.
