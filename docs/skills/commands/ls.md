Command skill: altaika ls

Use when you need to discover catalogs, schemas, tables, or columns without
reading row data.

Best path:
1. Start broad with `altaika ls`.
2. Narrow to `altaika ls <catalog>/<schema>`.
3. Use `--long` only when profile hints are worth the extra work.

If results include confusing internals, file an issue with the path and engine.
