# Leadline promo — 30-second video (Remotion)

1920×1080 @30fps, 900 frames, dark developer-terminal aesthetic.
Every visual is code-driven (terminal windows, syntax-highlighted code,
metric counters, SVG diagrams) — no screenshot assets required.
Typeface is JetBrains Mono throughout (via `@remotion/google-fonts`;
rendering fetches it from Google Fonts, with the system monospace stack
as fallback).

## Preview

```bash
npm install
npm run dev        # Remotion Studio at http://localhost:3000
```

## Render

```bash
npx remotion render src/index.ts LeadlinePromo out/leadline-promo.mp4
```

The published copy lives at `assets/leadline-promo.mp4` (with poster
`assets/promo-poster.png`, both embedded in the root `README.md`). After
re-rendering, copy the new file over and regenerate the poster:

```bash
cp out/leadline-promo.mp4 ../assets/leadline-promo.mp4
npx remotion still src/index.ts LeadlinePromo ../assets/promo-poster.png --frame=885
```

## Verify a single frame

```bash
npx remotion still src/index.ts LeadlinePromo out/still.png --frame=350
```

Useful frames: `90` (problem), `230` (analyze output), `380` (regression),
`555` (improvement), `710` (gate passed), `770` (agent loop), `885` (brand).

## Scene map

| Frames | Time | Scene | On-screen text |
|---|---|---|---|
| 0–110 | 0:00–0:04 | Problem: agent session stream, freeze, punchlines | THE CODE CHANGED. / DID IT GET BETTER? |
| 110–250 | 0:04–0:08 | Agent runs `leadline changed --base origin/main --format agent-json`, JSON reveal | ANALYZE THE CHANGE. |
| 250–390 | 0:08–0:13 | Nested `processPayment`, metrics count up (red) | SEE WHAT GOT WORSE. |
| 390–530 | 0:13–0:18 | Code morphs flat, metrics count down (green), drawn checks | REFACTOR. MEASURE. REPEAT. |
| 530–670 | 0:18–0:22 | Gate fail (shake) → wipe → pass, LOCAL·CI·**AGENTS** | GATE REGRESSIONS BEFORE THEY LAND. |
| 670–810 | 0:22–0:27 | Agent loop: MCP `analyze`/`check` calls, iteration 1 (red) → iteration 2 (green), MCP SERVER · AGENT-JSON · AGENT SKILL | BUILT FOR THE AGENT LOOP. |
| 810–900 | 0:27–0:30 | Logo drop, ripples, LEADLINE, `leadline analyze .` | Measure the change. |

Cuts are sharp by design: scenes are conditionally rendered by frame range,
no `TransitionSeries`, so `durationInFrames` is exactly 900 with no overlap
math and no trailing black frames.

## Voiceover timing

| Time | Line |
|---|---|
| 0:00–0:04 | AI can change code in seconds. But did the code actually get better? |
| 0:04–0:09 | Leadline analyzes the change — not just the final code. |
| 0:09–0:14 | See exactly where complexity, nesting, and risk increased. |
| 0:14–0:19 | Refactor, run it again, and verify that the change improved maintainability. |
| 0:19–0:24 | Catch regressions locally, in CI, or inside your coding-agent loop. |
| 0:24–0:27 | And because Leadline is agent-ready, the feedback becomes part of the loop. |
| 0:27–0:30 | Leadline. Measure the change. |

## Sound cue sheet (frames @30fps)

No audio is bundled — add these in the edit:

- 0–72: subtle keyboard ticks under the agent stream
- 84: low impact hit on DID IT GET BETTER?
- 122–183: typing ticks under the `changed` command
- 206: short warning pulse on `regressions: 1`
- 438–498: upward tonal resolution as metrics fall
- 625: soft click on the gate-pass wipe
- 670–810: subdued digital pulse under the agent loop
- 810: deep, clean logo hit on the logo drop
- 895: single cursor-blink tick on the final hold

## Notes

- All CLI invocations shown (`changed --base`, `check --cognitive/--cyclomatic/--max-nesting`,
  MCP `analyze`/`check` tools, `analyze .`) match the real argument parser in
  `src/main.rs` and the MCP server in `src/mcp.rs`.
- The `changed` JSON envelope is a dramatized summary for the video, not the
  literal `agent-json` schema (see `docs/changed-code.md` and `docs/json-schema.md`).
- Typography is JetBrains Mono (see header); the agent surfaces named in
  scene 6 (MCP server, agent-json, agent skill) correspond to `leadline mcp`,
  `--format agent-json`, and `integrations/common/leadline-skill`.
- Logo mark mirrors `assets/logo.svg` (sounding line + waves).
