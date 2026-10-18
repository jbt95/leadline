import { IsometricScene } from "./components/IsometricScene";
import { installCommand, TerminalHero } from "./components/TerminalHero";

const GITHUB = "https://github.com/jbt95/leadline";
const README = "https://github.com/jbt95/leadline#readme";

const LANGUAGES =
  "Go, Java, JavaScript, C, C++, TypeScript, TSX, Rust, Python, and Zig";

const METRICS = [
  { name: "Cyclomatic", note: "McCabe basis per function; language-specific branch rules" },
  { name: "Cognitive", note: "Nesting-weighted control flow" },
  { name: "Max nesting", note: "Starts at zero inside the function body" },
  { name: "Parameters", note: "Direct grammar parameter nodes" },
  { name: "Physical / logical LOC", note: "Inclusive range vs declaration and control statements" },
  { name: "Halstead", note: "Operators and operands from parsed tokens" },
  { name: "Maintainability", note: "Derived index from volume and complexity" },
  { name: "CRAP", note: "With LCOV, JaCoCo, or Cobertura when you pass a report" },
];

const STEPS = [
  {
    title: "Measure",
    body: "Run leadline changed with agent-json. One regression object names the function and the before/after cognitive delta.",
  },
  {
    title: "Fix",
    body: "Refactor or simplify the flagged function. Leadline parses with Tree-sitter; it never executes your code.",
  },
  {
    title: "Prove",
    body: "Run changed again. regressions: [] means the complexity delta is clean — your tests still prove behaviour.",
  },
];

const SURFACES = [
  {
    title: "CLI",
    cmd: "leadline analyze .",
    detail: "Terminal, JSON, SARIF, agent-json. Deterministic, offline, gitignore-aware discovery.",
  },
  {
    title: "MCP (read-only)",
    cmd: "leadline mcp",
    detail:
      "Twenty-one analyzer tools over stdio or HTTP. No caller-supplied code; filesystem reads only inside analysis paths.",
  },
  {
    title: "Skills & hooks",
    cmd: "leadline changed --format agent-json",
    detail:
      "Point harnesses at integrations/common/leadline-skill/SKILL.md. Post-edit hooks in warn mode.",
  },
  {
    title: "Local stats panel",
    cmd: "leadline stats",
    detail: "Loopback page and GET /api/project — same JSON as project --json. Read-only for the repo.",
  },
];

const FAQ = [
  {
    q: "Does leadline replace my linter or Sonar?",
    a: "No. It reports function-level complexity and related metrics. Quality gates use thresholds you set; it does not run your test suite or prove correctness.",
  },
  {
    q: "Does it execute my code?",
    a: "No. Tree-sitter parsing only — no build, no runtime, no project scripts, no network during analysis.",
  },
  {
    q: "Will the agent fix itself?",
    a: "Leadline surfaces regressions in agent-json. What the harness does with that signal is up to you and your hooks; there is no guarantee of self-correction.",
  },
  {
    q: "What languages are supported?",
    a: LANGUAGES + ".",
  },
];

function Header() {
  return (
    <header class="relative z-10 border-b border-rule/80">
      <div class="mx-auto flex max-w-6xl items-center justify-between gap-4 px-4 py-4 sm:px-6">
        <a href="#" class="flex items-center gap-3 no-underline">
          <img src="./logo.svg" alt="" width="36" height="36" class="rounded-lg" />
          <span class="font-mono text-sm text-ink">leadline</span>
        </a>
        <nav class="flex items-center gap-4 font-mono text-xs sm:text-sm">
          <a href="#surfaces" class="text-muted hover:text-accent">Surfaces</a>
          <a href="#faq" class="text-muted hover:text-accent">FAQ</a>
          <a
            href={GITHUB}
            class="rounded border border-rule px-3 py-1.5 text-accent hover:border-accent"
          >
            GitHub
          </a>
        </nav>
      </div>
    </header>
  );
}

export default function App() {
  return (
    <div class="relative min-h-dvh">
      <div class="noise" aria-hidden="true" />
      <Header />
      <main id="main" class="relative z-10">
        {/* Hero */}
        <section class="mx-auto grid max-w-6xl gap-10 px-4 py-12 sm:px-6 lg:grid-cols-2 lg:items-center lg:py-16">
          <div>
            <p class="font-mono text-xs uppercase tracking-widest text-accent">beta · MIT</p>
            <h1 class="mt-3 font-mono text-2xl font-medium leading-tight text-ink sm:text-3xl lg:text-4xl">
              Measure complexity before the PR.
            </h1>
            <p class="mt-4 max-w-lg font-mono text-sm leading-relaxed text-muted">
              Leadline measures complexity. It doesn&rsquo;t verify behaviour &mdash; your tests still
              own that.
            </p>
            <p class="mt-3 max-w-lg font-mono text-sm text-muted">
              Local, deterministic function-level analysis for {LANGUAGES}. Parse with Tree-sitter;
              never execute code.
            </p>
            <div class="mt-6 flex flex-wrap items-center gap-3">
              <a
                href={GITHUB}
                class="inline-flex items-center rounded border border-accent bg-accent-dim px-4 py-2 font-mono text-sm text-accent no-underline hover:bg-accent/20"
              >
                View on GitHub
              </a>
              <a
                href="https://github.com/jbt95/leadline/blob/main/docs/installation.md"
                class="font-mono text-xs text-muted underline hover:text-accent"
              >
                Install options
              </a>
            </div>
            <p class="mt-4 font-mono text-[11px] text-muted">
              <span class="text-accent">$</span>{" "}
              <code class="break-all text-ink">{installCommand}</code>
            </p>
          </div>
          <div class="min-w-0">
            <TerminalHero />
          </div>
        </section>

        {/* Loop + isometric */}
        <section class="border-t border-rule bg-panel/40">
          <div class="mx-auto grid max-w-6xl gap-10 px-4 py-14 sm:px-6 lg:grid-cols-2 lg:items-center">
            <div>
              <h2 class="font-mono text-lg text-ink">The agent loop</h2>
              <p class="mt-2 font-mono text-sm text-muted">
                Per-function deltas in agent-json — evidence for maintainability, not a substitute
                for tests.
              </p>
              <ol class="mt-8 space-y-6">
                {STEPS.map((s, i) => (
                  <li class="flex gap-4">
                    <span
                      class="flex h-8 w-8 shrink-0 items-center justify-center rounded border border-rule font-mono text-xs text-accent"
                      aria-hidden="true"
                    >
                      {i + 1}
                    </span>
                    <div>
                      <h3 class="font-mono text-sm text-ink">{s.title}</h3>
                      <p class="mt-1 font-mono text-xs leading-relaxed text-muted">{s.body}</p>
                    </div>
                  </li>
                ))}
              </ol>
            </div>
            <IsometricScene />
          </div>
        </section>

        {/* Metrics */}
        <section class="border-t border-rule">
          <div class="mx-auto max-w-6xl px-4 py-14 sm:px-6">
            <h2 class="font-mono text-lg text-ink">Per-function metrics</h2>
            <p class="mt-2 max-w-2xl font-mono text-sm text-muted">
              From the{" "}
              <a href="https://github.com/jbt95/leadline/blob/main/docs/metrics.md" class="text-accent underline">
                metric specification
              </a>
              . Nested functions are scored independently.
            </p>
            <ul class="mt-8 grid gap-3 sm:grid-cols-2">
              {METRICS.map((m) => (
                <li
                  class="rounded border border-rule bg-card px-4 py-3 font-mono text-xs"
                >
                  <span class="text-accent">{m.name}</span>
                  <span class="mt-1 block text-muted">{m.note}</span>
                </li>
              ))}
            </ul>
          </div>
        </section>

        {/* Surfaces */}
        <section id="surfaces" class="border-t border-rule bg-panel/40">
          <div class="mx-auto max-w-6xl px-4 py-14 sm:px-6">
            <h2 class="font-mono text-lg text-ink">Surfaces</h2>
            <p class="mt-2 font-mono text-sm text-muted">
              Same analyzer core — different wires for humans, CI, and harnesses.
            </p>
            <div class="mt-8 grid gap-6 lg:grid-cols-2">
              {SURFACES.map((s) => (
                <article class="rounded border border-rule bg-card p-5">
                  <h3 class="font-mono text-sm text-accent">{s.title}</h3>
                  <p class="mt-2 font-mono text-[11px] text-ink">
                    <span class="text-muted">$</span> {s.cmd}
                  </p>
                  <p class="mt-2 font-mono text-xs leading-relaxed text-muted">{s.detail}</p>
                </article>
              ))}
            </div>
            <div class="mt-10 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              <figure class="overflow-hidden rounded border border-rule">
                <img
                  src="./stats-overview.png"
                  alt="leadline stats: quality gate, reliability, coverage, policy, top risk, language mix"
                  class="w-full"
                  loading="lazy"
                  width="800"
                  height="450"
                />
              </figure>
              <figure class="overflow-hidden rounded border border-rule">
                <img
                  src="./stats-telemetry.png"
                  alt="leadline stats telemetry: invocation badges and per-operation p90 timeseries"
                  class="w-full"
                  loading="lazy"
                  width="800"
                  height="450"
                />
              </figure>
              <figure class="overflow-hidden rounded border border-rule sm:col-span-2 lg:col-span-1">
                <img
                  src="./stats-hotspots.png"
                  alt="leadline stats hotspots: risk treemap, churn scatter, component bars"
                  class="w-full"
                  loading="lazy"
                  width="800"
                  height="450"
                />
              </figure>
            </div>
          </div>
        </section>

        {/* Comparison */}
        <section class="border-t border-rule">
          <div class="mx-auto max-w-6xl px-4 py-14 sm:px-6">
            <h2 class="font-mono text-lg text-ink">Where it fits</h2>
            <div class="mt-6 max-w-3xl font-mono text-sm leading-relaxed text-muted">
              <p>
                Leadline is a single binary: local, deterministic, no hosted dashboard required.
                You run it in the repo; output is terminal, JSON, SARIF, or agent-json.
              </p>
              <p class="mt-4">
                Full platforms add execution, hosting, and policy ecosystems. Leadline answers a
                narrower question after an edit: did maintainability metrics move the wrong way on
                the functions you changed?
              </p>
            </div>
          </div>
        </section>

        {/* FAQ */}
        <section id="faq" class="border-t border-rule bg-panel/40">
          <div class="mx-auto max-w-6xl px-4 py-14 sm:px-6">
            <h2 class="font-mono text-lg text-ink">FAQ</h2>
            <dl class="mt-8 space-y-6">
              {FAQ.map((item) => (
                <div class="border-b border-rule pb-6 last:border-0">
                  <dt class="font-mono text-sm text-ink">{item.q}</dt>
                  <dd class="mt-2 font-mono text-xs leading-relaxed text-muted">{item.a}</dd>
                </div>
              ))}
            </dl>
          </div>
        </section>
      </main>

      <footer class="relative z-10 border-t border-rule">
        <div class="mx-auto flex max-w-6xl flex-col gap-2 px-4 py-8 font-mono text-xs text-muted sm:flex-row sm:items-center sm:justify-between sm:px-6">
          <p>
            <a href={GITHUB} class="text-accent underline">leadline</a> ·{" "}
            <a href="https://github.com/jbt95/leadline/blob/main/LICENSE" class="underline hover:text-accent">
              MIT License
            </a>
          </p>
          <p>
            <a href={README} class="underline hover:text-accent">Documentation</a>
          </p>
        </div>
      </footer>
    </div>
  );
}
