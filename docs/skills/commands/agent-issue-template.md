Command skill: altaika agent issue-template

Use when an agent finds a CLI bug, confusing output, bad help, or wrong SQL
translation and needs to prepare a GitHub issue.

Best path:
1. Run the smallest command that reproduces the problem.
2. Run `altaika agent issue-template`.
3. Fill every placeholder.
4. Remove secrets before filing.

This command prepares issue content only. It does not call GitHub.
