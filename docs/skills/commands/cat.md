Command skill: altaika cat

Use when you need bounded rows from a known table.

Best path:
1. Run `altaika describe <path>` first.
2. Pass `--columns` to avoid noisy row payloads.
3. Keep `--limit` small unless the user asked for more.
4. Read `meta.row_count` and `meta.columns`; run `altaika describe <path>` for the full source profile.

If filtering or path resolution is wrong, file an issue with the full command.
