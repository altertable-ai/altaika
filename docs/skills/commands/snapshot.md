Command skill: altaika snapshot

Use when an agent needs a local Parquet copy plus a JSON manifest.

Best path:
1. Describe the source table first.
2. Select only useful columns.
3. Keep the snapshot bounded with `--limit`.
4. Read the manifest before using the Parquet file.

If the manifest and data disagree, file an issue with both paths.
