Command skill: altaika auth

Use before remote work to check whether runtime credentials are present.

Best path:
1. Run `altaika --engine altertable auth`.
2. Read `missing` and set only the missing variables.
3. Run `altaika --engine altertable auth --check` when credentials are present.

If `auth` says ready but a handshake fails, file an issue with both outputs.
