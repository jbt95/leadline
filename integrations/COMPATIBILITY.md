# Harness Compatibility Matrix

> Architectural overview, not a support promise. Harness APIs drift quickly:
> re-check the vendor docs before relying on any row. Last checked 2026-09-12.

| Harness | Direct CLI | MCP | Skills / instructions | Hooks | Native plugin / extension | Secret gate | Recommended official integration | Supported versions (checked 2026-09-12) |
|---|---|---|---|---|---|---|---|---|
| Claude Code | Yes | Yes | Yes | Yes | Yes | manual tool + pre-commit staged hook (secret scan removed from the Stop hook; unavailable scanner warns non-blocking) | Plugin + MCP + skill + optional hooks | Not yet validated live |
| Pi | Yes | Via extension if desired | Yes | Extension events | Yes | warn after edit (post-edit warning; block via the explicit tool or pre-commit hook) | Native extension + skill | Not yet validated live |
| OMP | Yes | Yes / interoperability | Yes | Yes | Yes | block via the explicit tool (findings fail the call); shared runner and pre-commit hook work wherever git does | Native extension + skill, MCP fallback | Not yet validated live |
| OpenCode | Yes | Yes | Instructions | Plugin hooks | Yes | manual tool (`leadline_secret_check`, warn mode on demand) | Plugin (v1; v2 experimental) + MCP | Not yet validated live |
| Codex | Yes | Yes | AGENTS / skills | Harness-dependent | Evolving | manual tool (shared runner and pre-commit hook work wherever git does) | MCP + skill / instructions | Not yet validated live |
| Gemini CLI | Yes | Yes | Yes | Yes | Extension | block on findings (AfterAgent exit 2); unavailable scanner warns non-blocking | Gemini extension | Not yet validated live |
| Cursor | Yes | Yes | Rules / AGENTS | Limited, native evolution | — | manual tool (shared runner and pre-commit hook work wherever git does) | MCP + rule | Not yet validated live |
| Cline | Yes | Yes | Instructions | Yes | Yes | shared gate wrapper (exit `2` on findings or an unavailable scanner); blocking behavior unverified | MCP + optional plugin | Not yet validated live |
| Windsurf | Yes | Yes | Rules / instructions | Harness-dependent | — | manual tool (shared runner and pre-commit hook work wherever git does) | MCP | Not yet validated live |
| GitHub Copilot | Surface-dependent | Yes | Yes | Surface-dependent | Custom agents | manual tool (shared runner and pre-commit hook work wherever git does) | MCP + instructions / custom agent | Not yet validated live |

## Notes

- Every officially supported integration must pin its validated harness
  version in its own directory once integration contract tests run against
  a live harness release. Until then, all rows above are architectural.
- "Direct CLI" means the harness can run `leadline` as a shell command.
- "MCP" means the harness can consume the `leadline mcp` server (stdio by default, HTTP with `--port`).
- Prefer MCP where supported, direct CLI where shell execution exists,
  reusable instructions everywhere, and a native plugin only when it
  materially improves UX (lower context use, automatic changed-code
  feedback, safer permissions, better install).
- Cloud agent surfaces cannot reach a local stdio MCP server; use
  checked-in instructions plus direct CLI calls there.
