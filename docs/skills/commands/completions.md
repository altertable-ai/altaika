Command skill: altaika completions

Purpose:
Generate a shell completion script for the Altaika CLI.

Use when:
- Setting up a local terminal for faster command discovery.
- Giving an agent shell-aware completion metadata.

Examples:
```bash
altaika completions zsh
altaika completions bash
altaika completions fish
```

Notes:
- The output is a shell script, not JSON.
- Redirect it into your shell completion location when installing it permanently.
