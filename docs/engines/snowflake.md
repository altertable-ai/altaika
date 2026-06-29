# Snowflake Engine

Status: planned after v0.

Start with the Snowflake SQL API and metadata from `information_schema`. Native metadata can be added later when it improves planning or profile quality.

Required profile fields:

- account or host
- warehouse
- database
- schema
- role
- auth method

Reuse Snowflake's native connection profile and environment variable conventions. Do not implement Snowflake auth flows before execution exists.

The engine should make Snowflake usable through Altaika's typed `ls`, `describe`, and bounded `cat` commands while keeping explicit raw SQL behind `altaika sql`.
