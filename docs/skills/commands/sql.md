Command skill: altaika sql

Use raw SQL only when typed commands cannot express the task.

Best path:
1. Prefer `describe` and `cat` first.
2. Keep `--limit` unless the user asks for full results.
3. Use explicit catalog and schema names for remote engines.
4. Read `meta.row_count` and `meta.columns`; `meta.limit` is the CLI safety cap, not proof that the SQL query returned that many rows.

If SQL output is surprising, file an issue with query, engine, and JSON result.
