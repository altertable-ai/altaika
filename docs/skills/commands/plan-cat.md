Command skill: altaika plan cat

Use when you need bounded read SQL for another dialect.

Best path:
1. Pass a full table path.
2. Add `--columns` before filters when possible.
3. Keep `--limit` explicit.

If translation changes intent, file an issue with canonical and rendered SQL.
