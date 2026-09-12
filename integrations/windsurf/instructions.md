# leadline instructions for Windsurf Cascade

Paste into `.windsurfrules` or the project's Cascade rules. MCP only:
there is no dedicated leadline adapter for Windsurf.

Use the leadline analyzer for complexity feedback. The analyzer is read-only.
It never edits source code.

- After a substantial edit batch, run changed-code analysis:
  `leadline changed --base origin/main --json`
- To inspect one function: `leadline function <file> <name> --json`
- Prioritize functions changed by the current edit. Do not refactor
  unrelated legacy code solely to improve metrics.
- Never game metrics or sacrifice correctness to lower a score.
- After complexity-driven refactoring, run the project's tests.
