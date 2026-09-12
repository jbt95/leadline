# Harness Compatibility Matrix

> Architectural overview, not a support promise. Harness APIs drift quickly:
> re-check the vendor docs before relying on any row. Last checked 2026-09-12.

| Harness | Direct CLI | MCP | Skills / instructions | Hooks | Native plugin / extension | Recommended official integration | Supported versions (checked 2026-09-12) |
|---|---|---|---|---|---|---|---|
| Claude Code | Yes | Yes | Yes | Yes | Yes | Plugin + MCP + skill + optional hooks | Not yet validated live |
| Pi | Yes | Via extension if desired | Yes | Extension events | Yes | Native extension + skill | Not yet validated live |
| OMP | Yes | Yes / interoperability | Yes | Yes | Yes | Native extension + skill, MCP fallback | Not yet validated live |
| OpenCode | Yes | Yes | Instructions | Plugin hooks | Yes | Plugin (v1; v2 experimental) + MCP | Not yet validated live |
| Codex | Yes | Yes | AGENTS / skills | Harness-dependent | Evolving | MCP + skill / instructions | Not yet validated live |
| Gemini CLI | Yes | Yes | Yes | Yes | Extension | Gemini extension | Not yet validated live |
| Cursor | Yes | Yes | Rules / AGENTS | Limited, native evolution | — | MCP + rule | Not yet validated live |
| Cline | Yes | Yes | Instructions | Yes | Yes | MCP + optional plugin | Not yet validated live |
| Windsurf | Yes | Yes | Rules / instructions | Harness-dependent | — | MCP | Not yet validated live |
| GitHub Copilot | Surface-dependent | Yes | Yes | Surface-dependent | Custom agents | MCP + instructions / custom agent | Not yet validated live |

## Notes

- Every officially supported integration must pin its validated harness
  version in its own directory once integration contract tests run against
  a live harness release. Until then, all rows above are architectural.
- "Direct CLI" means the harness can run `leadline` as a shell command.
- "MCP" means the harness can consume the `leadline mcp` stdio server.
- Prefer MCP where supported, direct CLI where shell execution exists,
  reusable instructions everywhere, and a native plugin only when it
  materially improves UX (lower context use, automatic changed-code
  feedback, safer permissions, better install).
- Cloud agent surfaces cannot reach a local stdio MCP server; use
  checked-in instructions plus direct CLI calls there.
