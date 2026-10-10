# Agent skills

The 19 skills under [`skills/`](skills) are **vendored third-party content**,
committed so the repository is self-contained: an agent that clones it can
load them with no extra fetch.

## `skills-lock.json`

[`../skills-lock.json`](../skills-lock.json) is the provenance record for that
vendored tree. It is the output of whichever external skills manager installed
the files, not of anything in this repository, and this repository neither
writes nor reads it.

Each entry records, for one skill:

| Field | Meaning |
| --- | --- |
| `source` | GitHub `owner/repo` the skill came from |
| `sourceType` | Always `github` today |
| `skillPath` | Path of `SKILL.md` inside that repository |
| `computedHash` | Content hash, so drift in a vendored copy is detectable |

There are six sources across the 19 skills — `obra/superpowers` (14),
`sickn33/agentic-awesome-skills`, `jurgendn/agent-skills`,
`microsoft/skills`, `openai/skills`, and `vasilyu1983/ai-agents-public`.

Keep it in step with the tree: adding or removing a skill directory should be
reflected here, and a `computedHash` that no longer matches the vendored file
means the copy was edited locally. Refresh it by re-running the installing
tool rather than by hand — the exact command depends on which manager owns
the install, which is out of this repository's scope.

`Cargo.toml` excludes both `.agents/**` and `skills-lock.json` from the
published crate, so neither ships in a release.
