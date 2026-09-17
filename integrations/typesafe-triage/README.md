# leadline triage companion

Rank leadline JSON reports with [TypeSafe](https://docs.typesafe.ai) judgments
and print a prioritized worklist. This is a **companion**: leadline itself stays
offline, its MCP server stays read-only and network-free, and nothing here runs
unless you invoke it.

Every feature is **advisory**. It ranks and annotates; it never gates, never
changes an exit code, and never suppresses a finding — the deterministic report
stays the source of truth.

## Requirements

- Node 22.18 or newer (type stripping runs the TypeScript directly)
- No runtime dependencies; dev dependencies cover typechecking and linting
- A TypeSafe API key

## Setup

```console
export TYPESAFE_API_KEY="..."
```

| Variable | Default | Meaning |
| --- | --- | --- |
| `TYPESAFE_API_KEY` | — | Required. Sent as the bearer token. |
| `TYPESAFE_BASE_URL` | `https://api.typesafe.ai` | Gateway or mirror. |
| `TYPESAFE_DEFAULT_MODEL` | `jev-latest` | Model alias. |
| `TYPESAFE_TIMEOUT_MS` | `10000` | Per-request timeout. |

## Features

| Feature | Feed it | It ranks |
| --- | --- | --- |
| `changed` | `leadline changed --base REV --json` | Regressed and newly added functions, by delta size and judgment. |
| `check` | `leadline check … --json` | The whole backlog above thresholds; parse errors are must-fix notes. |
| `security` | `leadline security --sarif FILE --json` or `leadline vulnerabilities --osv/--trivy FILE --json` | Scanner findings by severity, exposure, and reachability. Findings are never suppressed. |
| `debt` | `leadline debt --base REV --json` | New function debt and increased risk; resolved and decreased rows are reported as wins. |
| `duplication` | `leadline duplication --json` | Clone groups as extraction candidates or intentional shapes. |
| `route` | a free-form request (argument or stdin) | The leadline workflow that fits, with the exact command and a confidence gate. |

## Usage

```console
leadline changed --base origin/main --json | \
  node integrations/typesafe-triage/bin/triage.ts changed

node integrations/typesafe-triage/bin/triage.ts security --input security.json
node integrations/typesafe-triage/bin/triage.ts debt --input debt.json --json
node integrations/typesafe-triage/bin/triage.ts route "did my last edit make things worse?"
```

| Flag | Meaning |
| --- | --- |
| `--input FILE` | Read the report from `FILE` instead of stdin. |
| `--json` | Machine-readable result instead of the terminal worklist. |
| `--dry-run` | Print the exact requests without calling TypeSafe. Works offline. |
| `--batch-size N` | Items per request (default 16, max 64). |

`node bin/triage.ts` runs without installing anything, and the package
declares a `leadline-triage` binary, so `npm link` exposes that command on
PATH.

`leadline-triage list` prints the feature catalog as JSON.

## Why judgments on top of deterministic reports?

Leadline tells you what is: complexity 31, up 19 since base, in a churned
file. It cannot tell you whether that function is a payment-critical path or
a throwaway script, whether its complexity is inherent to the problem or
accidental, or whether it deserves attention today versus acceptance as
standing debt. Those are semantic judgments over names, paths, and shapes —
brittle as hardcoded heuristics, routine for a calibrated model.

In practice the judgments buy three things:

- **A worklist order.** A 200-violation `check` backlog is unactionable;
  ranked by role, inherent complexity, and attention, the top rows are usually
  where to start. Nothing new is found — attention is spent where it
  compounds.
- **Reviewable accept-vs-act calls.** "Intentional boilerplate, 0.9
  confident" is a claim attached to a bucket, not a suppressed finding: the
  deterministic report still lists every row, so a wrong judgment costs one
  misranked item, never a missed issue.
- **Routing in plain language.** Mapping "did my last edit make things
  worse?" to the right leadline command is a semantic classification task;
  the confidence gate punts to you when unsure.

Judgments are calibrated estimates, not ground truth — validate weights and
thresholds on your own data before acting on them (see Limits). They also
cost latency, tokens, and a network call, which is why this stays a separate
opt-in tool: backlogs of three don't need it; backlogs of three hundred do.

## How ranking works

- Each code item gets three judgments: `role` (Choice), `inherent` complexity
  (Score), and `attention` (Noul). Risk rows in `debt` skip `inherent` (they
  carry no function metrics). Security adds `exposure` (Choice) and
  `reachable` (Noul); duplication judges `intent` (Choice); routing picks a
  `workflow` (Choice) and sends requests to the user (`ask_user`) when
  confidence is below 0.5.
- **Code owns the composition.** The deterministic delta (or severity) is
  normalized across one run, multiplied by the role weight, a floored
  inherent-complexity factor, and the attention probability. Weights and
  confidence thresholds live in `src/judgments.ts` and `src/compose.ts`, so
  recorded answers can be re-ranked without another inference call.
- **Buckets**: `act` (judgment is confident enough to act), `review` (act with
  caution: a judgment fell below the confidence bar), `accept` (standing debt
  or an intentional shape). The security feature never produces `accept`.
- Items are sent in bounded batches (16 by default); token usage per run is
  printed and returned in `--json`.

## Privacy

The companion sends only what leadline already reports: paths, function names,
metrics, scanner rule/advisory IDs and severities, and — for `route` — the
request text. Source text is never sent. The API key stays in the environment
and is never written anywhere; point `TYPESAFE_BASE_URL` at your own gateway
if policy requires it.

## Types, lint, and tests

```console
npm run check   # tsc --noEmit plus oxlint (the vendored anti-slop rules)
npm test        # node --test; offline, against a stub service, no key needed
```

The package enforces the MIT [anti-slop](https://github.com/dmmulroy/anti-slop)
Oxlint rules from `tools/anti-slop/` — see its `UPSTREAM.md` for the vendored
revision, the pinned `oxlint`/`@oxlint/plugins` versions, and how to update it.
The suite covers request construction (headers, body, timeout, key handling),
all six features' item extraction and composition, and end-to-end CLI runs
against a local stub.

## Limits

- Report shapes track leadline's JSON as of 0.9.x; a schema change means
  updating the matching feature module.
- Judgments are calibrated estimates, not ground truth. Validate thresholds
  and weights on your own data before acting on them
  (<https://docs.typesafe.ai/confidence>).
- Batches run sequentially: a large report becomes several requests.
- Requests are bounded: `--batch-size` caps at 64 items, `route` text caps at
  8000 characters, and no single outbound request body exceeds ~256 KB.
