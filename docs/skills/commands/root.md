Command skill: altaika

Use Altaika when an agent needs a small, structured data action from the
terminal. Prefer typed commands before raw SQL.

Best path:
1. Run `altaika auth` when using remote engines.
2. Run `altaika ls` to find tables.
3. Run `altaika describe <path>` before reading rows.
4. Use `altaika cat <path> --columns ... --limit ...` for samples.
5. Use `altaika skills list` to discover embedded command skills.

If behavior is wrong, file a GitHub issue with the command, engine, expected
JSON, actual JSON, and the smallest data fixture that reproduces it.
