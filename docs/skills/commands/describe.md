Command skill: altaika describe

Use when you need column names, types, and nullability before querying a table.

Best path:
1. Run `altaika describe <path>`.
2. Pick only needed columns for the next `cat` or `sql`.
3. Treat `profile: null` as schema-only metadata, not a failure.

If a column is missing or typed wrong, file an issue with the path and engine.
