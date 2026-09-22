import { ComponentProps } from "react";
import { memo, useMemo, useState } from "react";
import { AlertTriangle, CheckCircle2, XCircle } from "lucide-react";
import { Bar, BarChart, CartesianGrid, Line, LineChart, Tooltip, XAxis, YAxis } from "recharts";
import { ChartContainer, ChartTooltipContent, Donut, Histogram, TrendLine, CorrelationScatter, RiskTreemap, CouplingSankey, axisWidth, paletteColor, riskColor, useReducedMotion, type TrendDatum, type ScatterPoint } from "./charts";
import { Badge, Card, CardTitle, severityTone } from "./ui";
import { fmtInt, fmtPct, type CouplingSection, type MutationSummary, type Project, type RiskRow, type TelemetrySnapshot, type Violation } from "../lib/types";
import { quantile, mergeHistogram, type MergedHist } from "../lib/pure.mjs";

// ── Quality gate ─────────────────────────────────────────────────────────
// Threshold-free verdict over existing canonical values only.

export function Gate({ project }: { project: Project }) {
  const parseErrors = project.summary.parse_errors;
  const errorViolations = project.architecture_violations.filter(
    (v) => (v.severity ?? "").toLowerCase() === "error",
  ).length;
  const conditions = [
    { name: "Parse errors", value: parseErrors, hint: "summary.parse_errors" },
    { name: "Error policy violations", value: errorViolations, hint: "architecture_violations severity=error" },
  ];
  const failed = conditions.filter((c) => c.value > 0);
  const pass = failed.length === 0;
  return (
    <div className="verdict-enter overflow-hidden rounded border border-rulestrong bg-card">
      <p
        className={
          pass
            ? "m-0 flex items-center gap-2 border-b border-[#a9dcc0] bg-leafbg px-4 py-3 text-[15px] font-semibold text-[#14703c]"
            : "m-0 flex items-center gap-2 border-b border-[#f2b8bd] bg-bloodbg px-4 py-3 text-[15px] font-semibold text-blood"
        }
      >
        {pass ? <CheckCircle2 size={18} /> : <XCircle size={18} />}
        {pass
          ? "Quality Gate: Passed"
          : `Quality Gate: Failed (${failed.length} of ${conditions.length} conditions)`}
      </p>
      <ul className="m-0 list-none p-0">
        {conditions.map((c) => (
          <li
            key={c.name}
            title={c.hint}
            className="flex flex-wrap items-baseline gap-2 gap-x-4 border-t border-rule px-4 py-2 text-[13px] tabular-nums first:border-t-0"
          >
            <span className={c.value > 0 ? "inline-block h-2.5 w-2.5 flex-none rounded-full bg-blood" : "inline-block h-2.5 w-2.5 flex-none rounded-full bg-leaf"} />
            <span className="flex-[1_1_200px]">{c.name}</span>
            <span className="font-semibold">{fmtInt(c.value)}</span>
            <span className="text-xs text-muted">required 0</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

// ── Measure cards ────────────────────────────────────────────────────────

/** Human description for a store metric key. The raw key stays on the
    caption's title for PromQL users; visitors read the description. */
function metricCaption(key: string, fallback: string): { text: string; title: string } {
  const known: Record<string, string> = {
    leadline_invocations_total: "cumulative invocations",
    leadline_findings_total: "cumulative gate findings",
    leadline_live_cpu_millicores: "latest CPU gauge reading",
    leadline_live_rss_bytes: "latest memory gauge reading",
    'outcome="gate_failed"': "invocations with failed gates",
    "duration_seconds sum/count": "mean over the duration histogram",
    "summary.files": "files analyzed",
    "summary.functions": "functions analyzed",
    "summary.coverage_percent": "share of functions covered",
    "summary.mutation_score": "mutation testing score",
    "summary.duplicated_lines": "lines in clone groups",
    "summary.dependency_cycles": "import cycles found",
    "summary.dependency_edges": "import edges",
    "summary.duplication_groups": "clone groups",
    "summary.policy_violations": "policy violations",
    "summary.risk_model": "risk model",
  };
  return { text: known[key] ?? fallback, title: key };
}

function Measure({ label, value, sub, caption, tone, compact }: { label: string; value: string; sub?: string; caption: string; tone?: ComponentProps<typeof Badge>["tone"]; compact?: boolean }) {
  const described = metricCaption(caption, caption);
  return (
    <Card>
      <p className="m-0 text-xs text-muted">{label}</p>
      <div className="mt-1.5">
        <Badge tone={tone ?? "secondary"} className={compact ? "px-3 py-1 text-base" : "px-3 py-1 text-2xl"}>{value}</Badge>
      </div>
      {sub !== undefined && <p className="m-0 mt-1.5 text-xs text-muted">{sub}</p>}
      {described.text !== label && <p className="m-0 mt-2 text-xs text-muted" title={described.title}>{described.text}</p>}
    </Card>
  );
}

function Legend({ items }: { items: Array<{ label: string; value: string; color: string }> }) {
  return (
    <ul className="m-0 mt-2 flex list-none flex-wrap gap-1 gap-x-3 p-0 text-xs text-muted">
      {items.map((item) => (
        <li key={item.label}>
          <i className="mr-1.5 inline-block h-2.5 w-2.5 rounded-full" style={{ background: item.color }} />
          {item.label} · {item.value}
        </li>
      ))}
    </ul>
  );
}

const PAGE_SIZE = 200;

/** 200-row window over a sorted list, so a large repository never builds a
    million DOM nodes. Sorting and paging are presentation only. */
function usePager<T>(rows: T[]) {
  const [page, setPage] = useState(0);
  const pages = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
  const current = Math.min(page, pages - 1);
  return {
    slice: rows.slice(current * PAGE_SIZE, (current + 1) * PAGE_SIZE),
    page: current,
    pages,
    total: rows.length,
    setPage,
  };
}

/** Paging controls to sit under a windowed table. */
function Pager({ page, pages, total, onChange }: { page: number; pages: number; total: number; onChange: (page: number) => void }) {
  if (total <= PAGE_SIZE) return null;
  return (
    <p className="m-0 flex items-center gap-3 px-3 py-2 text-[11px] text-muted">
      <span>
        rows {page * PAGE_SIZE + 1}–{Math.min(total, (page + 1) * PAGE_SIZE)} of {total}
      </span>
      <button type="button" disabled={page === 0} onClick={() => onChange(page - 1)}>previous</button>
      <span>page {page + 1} / {pages}</span>
      <button type="button" disabled={page + 1 >= pages} onClick={() => onChange(page + 1)}>next</button>
    </p>
  );
}

export function Overview({ project }: { project: Project }) {
  const { summary, coverage } = project;
  const violations = project.architecture_violations;
  const sev = { error: 0, warning: 0, info: 0 };
  for (const v of violations) {
    const key = (v.severity ?? "info").toLowerCase();
    if (key === "error" || key === "warning") sev[key] += 1;
    else sev.info += 1;
  }
  const topRisk = [...project.risk.rows]
    .filter((r) => r.score != null)
    .sort((a, b) => (b.score ?? 0) - (a.score ?? 0))
    .slice(0, 5);
  const peak = Math.max(0, ...topRisk.map((r) => r.score ?? 0));

  return (
    <div className="grid grid-cols-[repeat(auto-fit,minmax(min(300px,100%),1fr))] gap-4">
      <Card>
        <CardTitle>Reliability · parse errors</CardTitle>
        <p className="m-0 mt-2 flex items-center gap-2 text-3xl font-semibold tabular-nums">
          <span className={summary.parse_errors > 0 ? "inline-block h-2.5 w-2.5 rounded-full bg-blood" : "inline-block h-2.5 w-2.5 rounded-full bg-leaf"} />
          {fmtInt(summary.parse_errors)}
        </p>
        <p className="m-0 mt-1 text-xs text-muted">
          {fmtInt(summary.files)} files · {fmtInt(summary.functions)} functions
        </p>
      </Card>

      <Card>
        <CardTitle>Coverage</CardTitle>
        {coverage != null && coverage.percent != null ? (
          <div className="mt-3 flex items-center gap-4">
            <Donut
              segments={[
                { label: "covered", value: coverage.covered_functions ?? 0, color: "#00a94f" },
                { label: "uncovered", value: Math.max(0, (coverage.functions ?? 0) - (coverage.covered_functions ?? 0)), color: "#e6e6e6" },
              ]}
            />
            <div>
              <p className="m-0 text-3xl font-semibold tabular-nums">{fmtPct(coverage.percent)}</p>
              <p className="m-0 mt-1 text-xs text-muted">
                {fmtInt(coverage.covered_functions)} of {fmtInt(coverage.functions)} functions
              </p>
            </div>
          </div>
        ) : (
          <p className="m-0 mt-2 text-xs text-muted">No coverage input: pass --lcov, --jacoco, or --coverage.</p>
        )}
      </Card>

      <Card>
        <CardTitle>Policy violations</CardTitle>
        {violations.length > 0 ? (
          <>
            <p className="m-0 mt-2 text-3xl font-semibold tabular-nums">{fmtInt(violations.length)}</p>
            <Legend
              items={[
                { label: "error", value: fmtInt(sev.error), color: "#d4333f" },
                { label: "warning", value: fmtInt(sev.warning), color: "#ed7d20" },
                { label: "info", value: fmtInt(sev.info), color: "#4b9fd5" },
              ]}
            />
          </>
        ) : (
          <>
            <p className="m-0 mt-2 text-3xl font-semibold tabular-nums">0</p>
            <p className="m-0 mt-1 text-xs text-muted">No architecture violations.</p>
          </>
        )}
      </Card>

      {topRisk.length > 0 && (
        <Card>
          <CardTitle>Top risk · score</CardTitle>
          <div className="mb-4 mt-3 grid gap-2">
            {topRisk.map((row) => (
              <div
                key={row.path ?? ""}
                title={`${row.path} · score ${row.score}`}
                className="grid grid-cols-[minmax(140px,auto)_minmax(0,1fr)_auto] items-center gap-3 text-[13px] tabular-nums"
              >
                <span className="min-w-0 max-w-[260px] overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs" title={row.path ?? ""}>
                  {row.path}
                </span>
                <span className="block h-2.5 w-full overflow-hidden rounded-full bg-rule">
                  <span className="block h-full bg-sky" style={{ width: `${peak > 0 ? (((row.score ?? 0) / peak) * 100).toFixed(1) : 0}%` }} />
                </span>
                <span>{(row.score ?? 0).toFixed(2)}</span>
              </div>
            ))}
          </div>
        </Card>
      )}

      <LanguageDonut files={project.files} />
    </div>
  );
}

export function LanguageDonut({ files }: { files: Project["files"] }) {
  const byLang = new Map<string, number>();
  for (const f of files) {
    const lang = f.language ?? "other";
    byLang.set(lang, (byLang.get(lang) ?? 0) + (f.loc ?? 0));
  }
  const langs = [...byLang.entries()].sort((a, b) => b[1] - a[1]);
  if (langs.length === 0) return null;
  const segments = langs.map(([label, value], i) => ({ label, value, color: paletteColor(i) }));
  return (
    <Card>
      <CardTitle>Code by language · lines</CardTitle>
      <div className="mt-3 flex items-center gap-4">
        <Donut segments={segments} />
        <div>
          <p className="m-0 text-3xl font-semibold tabular-nums">{langs[0][0]}</p>
          <p className="m-0 mt-1 text-xs text-muted">{fmtInt(langs[0][1])} lines · largest share</p>
        </div>
      </div>
      <Legend items={segments.map((s) => ({ label: s.label, value: fmtInt(s.value), color: s.color }))} />
    </Card>
  );
}

export function Coupling({ coupling }: { coupling: CouplingSection | null }) {
  if (coupling === null) {
    return <p className="m-0 rounded border border-rule bg-card px-4 py-3 text-[13px] text-muted">No Git history: coupling needs commit data.</p>;
  }
  if (!coupling.available) {
    return <p className="m-0 rounded border border-rule bg-card px-4 py-3 text-[13px] text-muted">{coupling.reason ?? "Coupling unavailable."}</p>;
  }
  const links = [...coupling.edges]
    .filter((e) => e.source != null && e.target != null && (e.co_changes ?? 0) > 0)
    .sort((a, b) => (b.co_changes ?? 0) - (a.co_changes ?? 0))
    .slice(0, 15)
    .map((e) => ({ source: e.source as string, target: e.target as string, value: e.co_changes ?? 0 }));
  return (
    <div className="flex flex-col gap-4">
      <Card>
        <p className="m-0 text-[13px] font-semibold">Files that change together</p>
        <p className="m-0 mt-1 text-xs text-muted">Top {links.length} co-change pairs · width: shared commits · no import connects them</p>
        <CouplingSankey links={links} />
      </Card>
      {links.length > 0 && (
        <Card className="overflow-x-auto p-0">
          <p className="m-0 px-3 pb-0 pt-3 text-[13px] font-semibold">Co-change pairs · ranked by shared commits</p>
          <table className="w-full border-separate border-spacing-0 text-[13px] tabular-nums">
            <thead>
              <tr>
                {["source", "target", "shared commits"].map((h) => (
                  <th key={h} className="border-b border-rulestrong bg-[#fafafa] px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-wide text-muted">
                    {h}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {links.map((l) => (
                <tr key={`${l.source}→${l.target}`} className="hover:[&>td]:bg-[#f7fafc]">
                  <td className="max-w-[280px] truncate border-b border-rule px-3 py-[7px] font-mono text-xs" title={l.source}>{l.source}</td>
                  <td className="max-w-[280px] truncate border-b border-rule px-3 py-[7px] font-mono text-xs" title={l.target}>{l.target}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtInt(l.value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
    </div>
  );
}

export function Measures({ project }: { project: Project }) {
  const s = project.summary;
  type Tone = ComponentProps<typeof Badge>["tone"];
  const items: Array<{ label: string; value: string; hint: string; tone: Tone; compact?: boolean }> = [
    { label: "files", value: fmtInt(s.files), hint: "summary.files", tone: "secondary" },
    { label: "functions", value: fmtInt(s.functions), hint: "summary.functions", tone: "secondary" },
    { label: "coverage", value: fmtPct(s.coverage_percent, 2), hint: "summary.coverage_percent", tone: s.coverage_percent == null ? "secondary" : "ok" },
    { label: "mutation score", value: s.mutation_score == null ? "—" : s.mutation_score.toFixed(2), hint: "summary.mutation_score", tone: s.mutation_score == null ? "secondary" : "ok" },
    { label: "duplicated lines", value: fmtInt(s.duplicated_lines), hint: "summary.duplicated_lines", tone: (s.duplicated_lines ?? 0) > 0 ? "warn" : "secondary" },
    { label: "clone groups", value: fmtInt(s.duplication_groups), hint: "summary.duplication_groups", tone: (s.duplication_groups ?? 0) > 0 ? "warn" : "secondary" },
    { label: "import edges", value: fmtInt(s.dependency_edges), hint: "summary.dependency_edges", tone: "secondary" },
    { label: "dependency cycles", value: fmtInt(s.dependency_cycles), hint: "summary.dependency_cycles", tone: (s.dependency_cycles ?? 0) > 0 ? "bad" : "secondary" },
    { label: "policy violations", value: fmtInt(s.policy_violations), hint: "summary.policy_violations", tone: (s.policy_violations ?? 0) > 0 ? "bad" : "secondary" },
    { label: "risk model", value: s.risk_model ?? "—", hint: "summary.risk_model", tone: "secondary", compact: true },
  ];
  const byLines = [...project.files]
    .filter((f) => (f.loc ?? 0) > 0)
    .sort((a, b) => (b.loc ?? 0) - (a.loc ?? 0));
  const pager = usePager(byLines);
  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(180px,100%),1fr))] gap-3">
        {items.map((item) => (
          <Measure key={item.label} label={item.label} value={item.value} caption={item.hint} tone={item.tone} compact={item.compact} />
        ))}
      </div>
      {byLines.length > 0 && (
        <Card className="overflow-x-auto p-0">
          <p className="m-0 px-3 pb-0 pt-3 text-[13px] font-semibold">Files by lines</p>
          <table className="w-full border-separate border-spacing-0 text-[13px] tabular-nums">
            <thead>
              <tr>
                {["file", "lines", "functions", "max cogn.", "coverage"].map((h) => (
                  <th key={h} className="border-b border-rulestrong bg-[#fafafa] px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-wide text-muted">
                    {h}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {pager.slice.map((f) => (
                <tr key={f.path} className="hover:[&>td]:bg-[#f7fafc]">
                  <td className="max-w-[280px] truncate border-b border-rule px-3 py-[7px] font-mono text-xs" title={f.path}>{f.path}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtInt(f.loc)}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtInt(f.functions)}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtInt(f.max_cognitive)}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtPct(f.coverage_percent)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <Pager page={pager.page} pages={pager.pages} total={pager.total} onChange={pager.setPage} />
        </Card>
      )}
    </div>
  );
}

// ── Hotspots ─────────────────────────────────────────────────────────────

const COMPONENT_COLORS: Record<string, string> = {
  complexity: "#4b9fd5",
  crap: "#1d75b3",
  churn: "#ed7d20",
  impact: "#eabe06",
  ownership: "#777777",
  policy: "#d4333f",
};

export function Hotspots({ rows, files }: { rows: RiskRow[]; files: Project["files"] }) {
  const scored = [...rows]
    .filter((r) => r.score != null)
    .sort((a, b) => (b.score ?? 0) - (a.score ?? 0));
  const pager = usePager(scored);
  const top = scored.slice(0, 8);
  if (scored.length === 0) return <p className="m-0 text-xs text-muted">No risk rows.</p>;
  const locByPath = new Map(files.map((f) => [f.path, f.loc ?? 1]));
  const treemapNodes = top.map((row) => ({
    name: (row.path ?? "?").split("/").pop() ?? "?",
    size: Math.max(1, locByPath.get(row.path ?? "") ?? 1),
    fill: riskColor(row.score ?? 0),
    detail: `${row.path} · score ${(row.score ?? 0).toFixed(1)}`,
  }));
  const churned = rows.filter((r) => r.changes != null && r.max_cognitive != null);
  const severityColor = (s: string | null) => {
    switch ((s ?? "").toLowerCase()) {
      case "error": return "#d4333f";
      case "warning": return "#ed7d20";
      case "info": return "#4b9fd5";
      default: return "#777777";
    }
  };
  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(380px,100%),1fr))] gap-4">
        <Card>
          <p className="m-0 text-[13px] font-semibold">Where risk concentrates</p>
          <p className="m-0 mt-1 text-xs text-muted">Area: lines of code · color: risk score, pale to red</p>
          <RiskTreemap nodes={treemapNodes} />
        </Card>
        <Card>
          <p className="m-0 text-[13px] font-semibold">Churn meets complexity</p>
          <p className="m-0 mt-1 text-xs text-muted">Horizontal: changes · vertical: max cognitive · size: blast radius · color: policy severity</p>
          <CorrelationScatter
            points={churned.map((r) => ({
              x: r.changes ?? 0,
              y: r.max_cognitive ?? 0,
              z: Math.max(1, r.blast_radius ?? 1),
              name: r.path ?? "?",
              detail: `${r.contributors ?? "—"} contributors · fan-in ${r.fan_in ?? "—"} · fan-out ${r.fan_out ?? "—"}`,
              color: severityColor(r.policy_severity),
            }))}
            xLabel="changes"
            yLabel="max cognitive"
          />
        </Card>
      </div>
      <Card>
      <CardTitle>Risk components</CardTitle>
      <div className="mb-4 mt-3 grid gap-2">
        {pager.slice.map((row) => {
          const parts = Object.entries(row.components ?? {})
            .filter(([key, v]) => v != null && (v as number) > 0 && key in COMPONENT_COLORS)
            .map(([key, v]) => ({ key, value: v as number }));
          const sum = parts.reduce((t, p) => t + p.value, 0);
          return (
            <div key={row.path ?? ""} title={`${row.path} · score ${row.score}`} className="grid grid-cols-[minmax(140px,auto)_minmax(0,1fr)_auto] items-center gap-3 text-[13px] tabular-nums">
              <span className="min-w-0 max-w-[260px] overflow-hidden text-ellipsis whitespace-nowrap font-mono text-xs" title={row.path ?? ""}>
                {row.path}
              </span>
              <span className="flex h-3 w-full overflow-hidden rounded-full bg-rule" role="img" aria-label={row.path ?? ""}>
                {parts.map((p) => (
                  <span
                    key={p.key}
                    className="block h-full"
                    style={{ width: `${sum > 0 ? ((p.value / sum) * 100).toFixed(1) : 0}%`, background: COMPONENT_COLORS[p.key] }}
                    title={`${p.key}: ${p.value.toFixed(2)}`}
                  />
                ))}
              </span>
              <span>{(row.score ?? 0).toFixed(2)}</span>
            </div>
          );
        })}
      </div>
      <Legend items={Object.entries(COMPONENT_COLORS).map(([label, color]) => ({ label, value: "", color }))} />
      <Pager page={pager.page} pages={pager.pages} total={pager.total} onChange={pager.setPage} />
      </Card>
    </div>
  );
}

// ── Complexity ───────────────────────────────────────────────────────────

export function Complexity({ project }: { project: Project }) {
  const cyclomatic = project.functions.map((f) => f.cyclomatic).filter((v): v is number => v != null);
  const cognitive = project.functions.map((f) => f.cognitive).filter((v): v is number => v != null);
  const langByPath = new Map(project.files.map((f) => [f.path, f.language ?? "other"]));
  const languages = [...new Set(langByPath.values())].sort();
  const points: ScatterPoint[] = project.functions
    .filter((f) => f.cyclomatic != null && f.cognitive != null)
    .map((f) => {
      const span = (f.end_line ?? f.start_line ?? 0) - (f.start_line ?? 0) + 1;
      const language = f.path != null ? (langByPath.get(f.path) ?? "other") : "other";
      return {
        x: f.cyclomatic ?? 0,
        y: f.cognitive ?? 0,
        z: Math.max(1, span),
        name: f.name ?? f.id ?? "?",
        detail: `${f.path} · ${span} lines · crap ${f.crap ?? "—"} · coverage ${f.coverage ?? "—"}`,
        color: paletteColor(languages.indexOf(language)),
      };
    });
  return (
    <div className="flex flex-col gap-4">
      <Card>
        <p className="m-0 text-[13px] font-semibold">Complexity outliers live top-right</p>
        <p className="m-0 mt-1 text-xs text-muted">Each point is a function · horizontal: cyclomatic · vertical: cognitive · size: lines · color: language</p>
        <CorrelationScatter points={points} xLabel="cyclomatic" yLabel="cognitive" height={280} />
      </Card>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(380px,100%),1fr))] gap-4">
        <Card>
          <p className="m-0 text-[13px] font-semibold">Cyclomatic · {fmtInt(project.functions.length)} functions</p>
          <Histogram values={cyclomatic} />
        </Card>
        <Card>
          <p className="m-0 text-[13px] font-semibold">Cognitive · {fmtInt(project.functions.length)} functions</p>
          <Histogram values={cognitive} />
        </Card>
      </div>
    </div>
  );
}

// ── Trends ───────────────────────────────────────────────────────────────

export function Trends({ project }: { project: Project }) {
  const snapshots = project.snapshots;
  if (!snapshots || snapshots.length === 0) {
    const s = project.summary;
    return (
      <div className="flex flex-col gap-4">
        <Card>
          <p className="m-0 text-[13px] font-semibold">Current position · the baseline history starts from</p>
          <p className="m-0 mt-1 text-xs text-muted">Today's analyzer values; pass --snapshots FILE to record them per commit and grow trends here.</p>
          <dl className="m-0 mt-3 grid grid-cols-[repeat(auto-fit,minmax(min(160px,100%),1fr))] gap-3">
            {[
              ["coverage", fmtPct(s.coverage_percent)],
              ["functions", fmtInt(s.functions)],
              ["duplicated lines", fmtInt(s.duplicated_lines)],
              ["dependency cycles", fmtInt(s.dependency_cycles)],
            ].map(([k, v]) => (
              <div key={k} className="rounded border border-rule bg-page px-3 py-2">
                <dt className="text-xs text-muted">{k}</dt>
                <dd className="m-0 mt-0.5 text-xl font-semibold tabular-nums">{v}</dd>
              </div>
            ))}
          </dl>
        </Card>
        <p className="m-0 rounded border border-rule bg-card px-4 py-3 text-[13px] text-muted">No snapshot history: pass --snapshots FILE.</p>
      </div>
    );
  }
  const ordered = [...snapshots].sort((a, b) => (a.commit_timestamp ?? 0) - (b.commit_timestamp ?? 0));
  const short = (c: string | null) => (c ? c.slice(0, 7) : "—");
  const series = (
    pick: (t: NonNullable<Project["snapshots"]>[number]) => number | null | undefined,
    fmt: (v: number) => string,
  ): TrendDatum[] =>
    ordered.map((t) => {
      const v = pick(t) ?? null;
      return { label: short(t.commit), value: v, shown: v == null ? "—" : fmt(v) };
    });
  const charts = [
    { title: "Coverage %", data: series((t) => t.coverage_percent, (v) => fmtPct(v)), fmt: (v: number) => `${v.toFixed(1)}%` },
    { title: "Max risk score", data: series((t) => t.max_risk_score, (v) => v.toFixed(2)), fmt: (v: number) => v.toFixed(2) },
    { title: "Functions", data: series((t) => t.functions, (v) => fmtInt(v)), fmt: (v: number) => Math.round(v).toLocaleString("en-US") },
    { title: "Duplicated lines", data: series((t) => t.duplicated_lines, (v) => fmtInt(v)), fmt: (v: number) => Math.round(v).toLocaleString("en-US") },
  ];
  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(340px,100%),1fr))] gap-4">
        {charts.map((c) => (
          <Card key={c.title}>
            <p className="m-0 text-[13px] font-semibold">{c.title} · {snapshots.length} snapshots</p>
            <TrendLine points={c.data} formatY={c.fmt} formatTooltip={c.fmt} />
          </Card>
        ))}
      </div>
      <Card className="overflow-x-auto p-0">
        <p className="m-0 px-3 pb-0 pt-3 text-[13px] font-semibold">Snapshot history · oldest first</p>
        <table className="w-full border-separate border-spacing-0 text-[13px] tabular-nums">
          <thead>
            <tr>
              {["commit", "coverage", "max risk", "functions", "dup lines"].map((h) => (
                <th key={h} className="border-b border-rulestrong bg-[#fafafa] px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-wide text-muted">
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {ordered.map((t, i) => (
              <tr key={`${t.commit ?? i}-${i}`} className="hover:[&>td]:bg-[#f7fafc]">
                <td className="border-b border-rule px-3 py-[7px] font-mono text-xs" title={t.commit ?? ""}>{short(t.commit)}</td>
                <td className="border-b border-rule px-3 py-[7px] text-right">{fmtPct(t.coverage_percent)}</td>
                <td className="border-b border-rule px-3 py-[7px] text-right">{t.max_risk_score == null ? "—" : t.max_risk_score.toFixed(2)}</td>
                <td className="border-b border-rule px-3 py-[7px] text-right">{fmtInt(t.functions)}</td>
                <td className="border-b border-rule px-3 py-[7px] text-right">{fmtInt(t.duplicated_lines)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
    </div>
  );
}

// ── Policy violations ────────────────────────────────────────────────────

export function Violations({ rows }: { rows: Violation[] }) {
  if (rows.length === 0) return <p className="m-0 text-xs text-muted">No architecture violations.</p>;
  const rank = (s: string | null) => (s ?? "").toLowerCase() === "error" ? 0 : (s ?? "").toLowerCase() === "warning" ? 1 : 2;
  const byRule = new Map<string, { count: number; worst: string | null }>();
  for (const row of rows) {
    const key = row.rule ?? "—";
    const entry = byRule.get(key) ?? { count: 0, worst: null };
    entry.count += 1;
    if (entry.worst === null || rank(row.severity) < rank(entry.worst)) entry.worst = row.severity;
    byRule.set(key, entry);
  }
  const rules = [...byRule.entries()]
    .map(([rule, e]) => ({ rule, ...e }))
    .sort((a, b) => b.count - a.count || rank(a.worst) - rank(b.worst));
  return (
    <div className="flex flex-col gap-4">
      <Card>
        <p className="m-0 text-[13px] font-semibold">Violations by rule · {rules.length} rules</p>
        <ul className="m-0 mt-3 flex list-none flex-col gap-2 p-0">
          {rules.map((r) => (
            <li key={r.rule} className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 text-[13px] tabular-nums">
              <span className="truncate font-mono text-xs" title={r.rule}>{r.rule}</span>
              <Badge tone={severityTone(r.worst)}>{r.worst ?? "—"}</Badge>
              <span className="font-semibold">{fmtInt(r.count)}</span>
            </li>
          ))}
        </ul>
      </Card>
    <Card className="overflow-x-auto p-0">
      <table className="w-full border-separate border-spacing-0 text-[13px] tabular-nums">
        <thead>
          <tr>
            {["rule", "severity", "source", "target"].map((h) => (
              <th key={h} className="border-b border-rulestrong bg-[#fafafa] px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-wide text-muted">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i} className="hover:[&>td]:bg-[#f7fafc]">
              <td className="border-b border-rule px-3 py-[7px] font-mono text-xs">{row.rule}</td>
              <td className="border-b border-rule px-3 py-[7px]">
                <Badge tone={severityTone(row.severity)}>{row.severity ?? "—"}</Badge>
              </td>
              <td className="max-w-[240px] overflow-hidden text-ellipsis whitespace-nowrap border-b border-rule px-3 py-[7px] font-mono text-xs" title={row.source ?? ""}>
                {row.source}
              </td>
              <td className="max-w-[240px] overflow-hidden text-ellipsis whitespace-nowrap border-b border-rule px-3 py-[7px] font-mono text-xs" title={row.target ?? ""}>
                {row.target}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </Card>
    </div>
  );
}

// ── Telemetry ────────────────────────────────────────────────────────────
// Local usage telemetry (CPU, memory, latency): the opt-in store behind
// LEADLINE_METRICS_DIR. Disabled unless the server process has it set.

function fmtSecs(value: number | null): string {
  if (value == null) return "—";
  return value < 1 ? `${Math.round(value * 1000)} ms` : `${value.toFixed(2)} s`;
}

function fmtCores(value: number | null): string {
  return value == null ? "—" : `${value.toFixed(2)} cores`;
}

/** Timeseries card with its own range selector. Ranges are presets, not a
    calendar: the server keeps one hour of 5-second samples in memory, so a
    date picker would promise history that does not exist. */
const RANGES = [
  { label: "5m", ms: 5 * 60_000 },
  { label: "15m", ms: 15 * 60_000 },
  { label: "1h", ms: 60 * 60_000 },
  { label: "All", ms: Number.POSITIVE_INFINITY },
];

function SeriesCard({ title, sub, points, formatY, formatTooltip }: { title: string; sub: string; points: TrendDatum[]; formatY?: (v: number) => string; formatTooltip?: (v: number) => string }) {
  const [range, setRange] = useState<number>(Number.POSITIVE_INFINITY);
  const active = RANGES.find((r) => r.ms === range) ?? RANGES[RANGES.length - 1];
  const visible =
    range === Number.POSITIVE_INFINITY ? points : points.filter((p) => (p.t ?? 0) >= Date.now() - range);
  return (
    <Card>
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <p className="m-0 text-[13px] font-semibold">{title}</p>
        <RangeButtons range={range} onChange={setRange} label={title} />
      </div>
      <p className="m-0 mt-1 text-xs text-muted">{sub}</p>
      <TrendLine points={visible} formatY={formatY} formatTooltip={formatTooltip} />
      <p className="m-0 mt-1 text-xs text-muted" aria-live="polite">
        Showing {visible.length} of {points.length} samples · {active.label === "All" ? "full retained hour" : `last ${active.label}`}
      </p>
    </Card>
  );
}

const OP_COLORS = ["#4b9fd5", "#ed7d20", "#00a94f", "#8172b3", "#eabe06", "#d4333f"];

/** Multi-line timeseries: one line per operation, sharing one range control. */
function MultiSeriesCard({ title, sub, rows, ops, formatY, formatTooltip }: {
  title: string;
  sub: string;
  rows: Array<Record<string, number | string | null>>;
  ops: string[];
  formatY?: (v: number) => string;
  formatTooltip?: (v: number) => string;
}) {
  const reduced = useReducedMotion();
  const [range, setRange] = useState<number>(Number.POSITIVE_INFINITY);
  const active = RANGES.find((r) => r.ms === range) ?? RANGES[RANGES.length - 1];
  const visible =
    range === Number.POSITIVE_INFINITY
      ? rows
      : rows.filter((r) => typeof r.t === "number" && r.t >= Date.now() - range);
  return (
    <Card>
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <p className="m-0 text-[13px] font-semibold">{title}</p>
        <RangeButtons range={range} onChange={setRange} label={title} />
      </div>
      <p className="m-0 mt-1 text-xs text-muted">{sub}</p>
      <ChartContainer
        config={Object.fromEntries(ops.map((op, i) => [op, { label: op, color: OP_COLORS[i % OP_COLORS.length] }]))}
        height={220}
      >
        <LineChart data={visible} margin={{ top: 12, right: 8, bottom: 0, left: 0 }}>
          <CartesianGrid vertical={false} stroke="#e6e6e6" />
          <XAxis dataKey="label" tickLine={false} axisLine={{ stroke: "#d4d4d4" }} tick={{ fontSize: 10, fill: "#777" }} minTickGap={32} />
          <YAxis
            tickLine={false}
            axisLine={false}
            tick={{ fontSize: 10, fill: "#777" }}
            tickFormatter={formatY ?? ((v: number) => `${v}`)}
            width={52}
            domain={["auto", "auto"]}
          />
          <Tooltip
            content={
              <ChartTooltipContent
                formatter={(v) => (typeof v === "number" && formatTooltip !== undefined ? formatTooltip(v) : `${v}`)}
              />
            }
          />
          {ops.map((op, i) => (
            <Line
              key={op}
              type="monotone"
              dataKey={op}
              stroke={OP_COLORS[i % OP_COLORS.length]}
              strokeWidth={2}
              dot={false}
              activeDot={{ r: 3 }}
              connectNulls={false}
              isAnimationActive={!reduced}
            />
          ))}
        </LineChart>
      </ChartContainer>
      <p className="m-0 mt-1 text-xs text-muted" aria-live="polite">
        Showing {visible.length} samples · {active.label === "All" ? "full retained hour" : `last ${active.label}`} · {ops.length} operations
      </p>
    </Card>
  );
}

function RangeButtons({ range, onChange, label }: { range: number; onChange: (ms: number) => void; label: string }) {
  return (
    <div className="flex overflow-hidden rounded border border-rulestrong" role="group" aria-label={`${label} timeframe`}>
      {RANGES.map((r) => (
        <button
          key={r.label}
          type="button"
          title={r.label === "All" ? "Show the full retained hour" : `Show the last ${r.label}`}
          aria-pressed={range === r.ms}
          onClick={() => onChange(r.ms)}
          className={
            range === r.ms
              ? "cursor-pointer border-0 bg-deep px-2 py-1 text-[11px] font-semibold text-white"
              : "cursor-pointer border-0 bg-card px-2 py-1 text-[11px] text-muted hover:text-deep"
          }
        >
          {r.label}
        </button>
      ))}
    </div>
  );
}

export const Telemetry = memo(function Telemetry({ snapshot, error, onRetry }: { snapshot: TelemetrySnapshot | null; error: string | null; onRetry: () => void }) {
  if (error !== null) {
    return (
      <div className="rounded border border-[#f2b8bd] bg-bloodbg px-4 py-3 text-[13px] text-blood">
        <p className="m-0">Telemetry unavailable: {error}</p>
        <button
          type="button"
          onClick={onRetry}
          className="mt-2 cursor-pointer rounded border border-blood bg-card px-2 py-1 text-xs font-semibold text-blood hover:bg-blood hover:text-white"
        >
          Try again
        </button>
      </div>
    );
  }
  if (snapshot == null) return <p className="m-0 text-xs text-muted">Loading telemetry…</p>;
  if (snapshot.status === "disabled") {
    return (
      <p className="m-0 rounded border border-rule bg-card px-4 py-3 text-[13px] text-muted">
        Telemetry is off: {snapshot.reason ?? "set LEADLINE_METRICS_DIR on the server to record CPU, memory, and latency."}
      </p>
    );
  }
  const model = useMemo(() => {
    const counters = snapshot?.counters ?? [];
    const summaries = snapshot?.summaries ?? [];
    const gauges = snapshot?.gauges ?? [];
    const invocations = counters.filter((r) => r.metric === "leadline_invocations_total");
    const total = invocations.reduce((t, r) => t + r.value, 0);
    const failed = invocations.filter((r) => r.labels["outcome"] === "gate_failed").reduce((t, r) => t + r.value, 0);
    const durations = summaries.filter((r) => r.metric === "leadline_invocation_duration_seconds");
    const byOp = new Map<string, { calls: number; latSum: number; latCount: number }>();
    for (const row of invocations) {
      const op = row.labels["operation"] ?? "other";
      const entry = byOp.get(op) ?? { calls: 0, latSum: 0, latCount: 0 };
      entry.calls += row.value;
      byOp.set(op, entry);
    }
    for (const row of durations) {
      const op = row.labels["operation"] ?? "other";
      const entry = byOp.get(op) ?? { calls: 0, latSum: 0, latCount: 0 };
      entry.latSum += row.sum;
      entry.latCount += row.count;
      byOp.set(op, entry);
    }
    const ops = [...byOp.entries()].sort((a, b) => b[1].calls - a[1].calls).slice(0, 12);
    const histFor = (metric: string, op: string) =>
      mergeHistogram(summaries.filter((r) => r.metric === metric && (r.labels["operation"] ?? "other") === op));
    const outcomeRows: Array<{ operation: string; success: number; gate_failed: number; other: number }> = ops.map(([operation]) => {
      const counts = { success: 0, gate_failed: 0, other: 0 };
      for (const row of invocations) {
        if ((row.labels["operation"] ?? "other") !== operation) continue;
        const outcome = row.labels["outcome"] ?? "other";
        if (outcome === "success") counts.success += row.value;
        else if (outcome === "gate_failed") counts.gate_failed += row.value;
        else counts.other += row.value;
      }
      return { operation, ...counts };
    });
    const tableRows = ops.map(([operation, e]) => {
      const dur = histFor("leadline_invocation_duration_seconds", operation);
      const cpu = histFor("leadline_invocation_cpu_ratio", operation);
      const cpuSec = histFor("leadline_invocation_cpu_seconds", operation);
      const rss = histFor("leadline_invocation_max_rss_bytes", operation);
      const pct = (m: MergedHist | null, q: number) =>
    m === null ? null : quantile(m.bounds, m.buckets, m.count, q);
  const pctMiB = (m: MergedHist | null, q: number) => {
    const v = pct(m, q);
    return v == null ? null : v / 1048576;
  };
      return {
        operation,
        calls: e.calls,
        p50: pct(dur, 0.5),
        p90: pct(dur, 0.9),
        p99: pct(dur, 0.99),
        cpuP50: pct(cpu, 0.5),
        cpuP90: pct(cpu, 0.9),
        cpuP99: pct(cpu, 0.99),
        cpuSecP50: pct(cpuSec, 0.5),
        cpuSecP90: pct(cpuSec, 0.9),
        cpuSecP99: pct(cpuSec, 0.99),
        rssP50: pctMiB(rss, 0.5),
        rssP90: pctMiB(rss, 0.9),
        rssP99: pctMiB(rss, 0.99),
      };
    });
    const liveCpu = gauges.filter((r) => r.metric === "leadline_live_cpu_millicores");
    const liveRss = gauges.filter((r) => r.metric === "leadline_live_rss_bytes");
    const findings = counters.filter((r) => r.metric === "leadline_findings_total");
    const findingGroups = new Map<string, number>();
    for (const row of findings) {
      const key = `${row.labels["kind"] ?? "?"} · ${row.labels["state"] ?? "?"}`;
      findingGroups.set(key, (findingGroups.get(key) ?? 0) + row.value);
    }
    const findingRows = [...findingGroups.entries()]
      .map(([label, value]) => ({ label, value }))
      .sort((a, b) => b.value - a.value)
      .slice(0, 12);
    const series = snapshot?.series ?? [];
    const clock = (t: number) => new Date(t).toLocaleTimeString();
    const cpuSeries = series.map((s) => ({
      label: clock(s.t_ms),
      t: s.t_ms,
      value: s.cpu_mc == null ? null : s.cpu_mc / 1000,
      shown: s.cpu_mc == null ? "—" : `${(s.cpu_mc / 1000).toFixed(2)} cores`,
    }));
    const rssSeries = series.map((s) => ({
      label: clock(s.t_ms),
      t: s.t_ms,
      value: s.rss_bytes == null ? null : s.rss_bytes / 1048576,
      shown: s.rss_bytes == null ? "—" : `${Math.round(s.rss_bytes / 1048576)} MiB`,
    }));
    const eventSeries = series.map((s, i) => {
      if (i === 0) return { label: clock(s.t_ms), t: s.t_ms, value: null as number | null, shown: "—" };
      const dt = (s.t_ms - series[i - 1].t_ms) / 1000;
      const dv = s.invocations - series[i - 1].invocations;
      const rate = dt > 0 ? (dv / dt) * 60 : null;
      return { label: clock(s.t_ms), t: s.t_ms, value: rate, shown: rate == null ? "—" : `${rate.toFixed(1)}/min` };
    });
    const topOp = ops.length > 0 ? ops[0][0] : null;
    const topDist = topOp === null ? null : histFor("leadline_invocation_duration_seconds", topOp);
    const distRows = topDist === null ? [] : topDist.bounds.map((bound, i) => {
      const prev = i > 0 ? topDist.buckets[i - 1] : 0;
      return { bucket: `≤ ${fmtSecs(bound)}`, count: Math.max(0, topDist.buckets[i] - prev) };
    });
    const bubblePoints: ScatterPoint[] = tableRows
      .filter((r) => r.cpuP90 != null && r.rssP90 != null)
      .map((r) => ({
        x: (r.cpuP90 ?? 0),
        y: (r.rssP90 ?? 0),
        z: Math.max(1, r.calls),
        name: r.operation,
        detail: `p90 latency ${fmtSecs(r.p90)} · ${fmtInt(r.calls)} calls`,
        color: "#4b9fd5",
      }));
    const heatMax = Math.max(1, ...outcomeRows.flatMap((r) => [r.success, r.gate_failed, r.other]));
    const lineOps = tableRows.slice(0, 6).map((r) => r.operation);
    const lineRows = (
      pick: (o: { latency_p90: number | null; cpu_p90: number | null; cpu_seconds_p90: number | null; rss_p90: number | null }) => number | null,
      scale: (v: number) => number,
    ) =>
      series.map((s) => {
        const row: Record<string, number | string | null> = { label: clock(s.t_ms), t: s.t_ms };
        for (const op of lineOps) {
          const entry = (s.ops ?? []).find((o) => o.operation === op);
          const raw = entry === undefined ? null : pick(entry);
          row[op] = raw == null ? null : scale(raw);
        }
        return row;
      });
    const latLines = lineRows((o) => o.latency_p90, (v) => v);
    const cpuLines = lineRows((o) => o.cpu_p90, (v) => v);
    const cpuSecLines = lineRows((o) => o.cpu_seconds_p90, (v) => v);
    const rssLines = lineRows((o) => o.rss_p90, (v) => v / 1048576);
    return { total, failed, durations, ops, outcomeRows, tableRows, liveCpu, liveRss, findings, findingRows, cpuSeries, rssSeries, eventSeries, topOp, distRows, bubblePoints, heatMax, lineOps, latLines, cpuLines, cpuSecLines, rssLines };
  }, [snapshot]);
  const { total, failed, durations, ops, outcomeRows, tableRows, liveCpu, liveRss, findings, findingRows, cpuSeries, rssSeries, eventSeries, topOp, distRows, bubblePoints, heatMax, lineOps, latLines, cpuLines, cpuSecLines, rssLines } = model;
  const reduced = useReducedMotion();

  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(180px,100%),1fr))] gap-3">
        <Measure label="invocations" value={fmtInt(total)} caption="leadline_invocations_total" tone="secondary" />
        <Measure label="gate failures" value={fmtInt(failed)} caption='outcome="gate_failed"' tone={failed > 0 ? "bad" : "secondary"} />
        <Measure
          label="mean latency"
          value={(() => {
            const sum = durations.reduce((t, r) => t + r.sum, 0);
            const count = durations.reduce((t, r) => t + r.count, 0);
            return count === 0 ? "—" : fmtSecs(sum / count);
          })()}
          caption="duration_seconds sum/count"
          tone="secondary"
        />
        <Measure
          label="live CPU util."
          value={liveCpu.length === 0 ? "—" : `${(Math.max(...liveCpu.map((r) => r.value)) / 1000).toFixed(2)} cores`}
          sub="last recorded invocation"
          caption="leadline_live_cpu_millicores"
          tone="secondary"
        />
        <Measure
          label="live memory"
          value={liveRss.length === 0 ? "—" : `${(Math.max(...liveRss.map((r) => r.value)) / 1048576).toFixed(0)} MiB`}
          sub="last recorded invocation"
          caption="leadline_live_rss_bytes"
          tone="secondary"
        />
        <Measure
          label="gate findings"
          value={fmtInt(findings.reduce((t, r) => t + r.value, 0))}
          caption="leadline_findings_total"
          tone={findings.length > 0 ? "warn" : "secondary"}
        />
      </div>

      {latLines.some((r) => lineOps.some((op) => r[op] != null)) ? (
        <MultiSeriesCard
          title="Latency p90 over time"
          sub="Seconds per invocation for the busiest operations · sampled every 5 s"
          rows={latLines}
          ops={lineOps}
          formatY={(v) => (v < 1 ? `${Math.round(v * 1000)}ms` : `${v.toFixed(1)}s`)}
          formatTooltip={(v) => fmtSecs(v)}
        />
      ) : (
        ops.length > 0 && (
          <p className="m-0 rounded border border-rule bg-card px-4 py-3 text-[13px] text-muted">
            No latency samples yet: duration histograms accumulate as commands run with LEADLINE_METRICS_DIR set.
          </p>
        )
      )}

      {cpuLines.some((r) => lineOps.some((op) => r[op] != null)) && (
        <MultiSeriesCard
          title="CPU utilization p90 over time"
          sub="Cores in use for the busiest operations · sampled every 5 s"
          rows={cpuLines}
          ops={lineOps}
          formatY={(v) => `${v.toFixed(1)}`}
          formatTooltip={(v) => `${v.toFixed(2)} cores`}
        />
      )}

      {rssLines.some((r) => lineOps.some((op) => r[op] != null)) && (
        <MultiSeriesCard
          title="Peak memory p90 over time"
          sub="Peak resident MiB for the busiest operations · sampled every 5 s"
          rows={rssLines}
          ops={lineOps}
          formatY={(v) => Math.round(v).toLocaleString("en-US")}
          formatTooltip={(v) => `${Math.round(v).toLocaleString("en-US")} MiB`}
        />
      )}

      {ops.length > 0 && (
        <Card>
          <p className="m-0 text-[13px] font-semibold">Calls by operation and outcome</p>
          <ChartContainer config={{ success: { label: "success", color: "#00a94f" }, gate_failed: { label: "gate failed", color: "#d4333f" }, other: { label: "other", color: "#e6e6e6" } }} height={200}>
            <BarChart data={outcomeRows} margin={{ top: 12, right: 8, bottom: 0, left: -8 }} layout="vertical">
              <CartesianGrid horizontal={false} stroke="#e6e6e6" />
              <XAxis type="number" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "#777" }} allowDecimals={false} tickFormatter={(v: number) => Math.round(v).toLocaleString("en-US")} />
              <YAxis type="category" dataKey="operation" tickLine={false} axisLine={false} tick={{ fontSize: 11, fill: "#333" }} width={axisWidth(outcomeRows.map((r) => r.operation))} />
              <Tooltip content={<ChartTooltipContent />} cursor={{ fill: "#f3f3f3" }} />
              <Bar dataKey="success" stackId="calls" fill="var(--color-success)" maxBarSize={22} isAnimationActive={!reduced} />
              <Bar dataKey="gate_failed" stackId="calls" fill="var(--color-gate_failed)" maxBarSize={22} isAnimationActive={!reduced} />
              <Bar dataKey="other" stackId="calls" fill="var(--color-other)" radius={[0, 3, 3, 0]} maxBarSize={22} isAnimationActive={!reduced} />
            </BarChart>
          </ChartContainer>
          <Legend items={[{ label: "success", value: "", color: "#00a94f" }, { label: "gate failed", value: "", color: "#d4333f" }, { label: "other", value: "", color: "#e6e6e6" }]} />
        </Card>
      )}

      {eventSeries.some((p) => p.value !== null) && (
        <div className="grid grid-cols-[repeat(auto-fit,minmax(340px,1fr))] gap-4">
          <SeriesCard title="CPU utilization over time" sub="Cores in use, from the sampler live gauges · 5 s samples, last hour" points={cpuSeries} formatY={(v) => `${v.toFixed(2)}`} formatTooltip={(v) => `${v.toFixed(2)} cores`} />
          <SeriesCard title="Sampled memory over time" sub="Resident MiB from the sampler live gauges · 5 s samples, last hour" points={rssSeries} formatY={(v) => Math.round(v).toLocaleString("en-US")} formatTooltip={(v) => `${Math.round(v).toLocaleString("en-US")} MiB`} />
          <SeriesCard title="Invocation rate" sub="Store-wide events per minute, from counter deltas" points={eventSeries} formatY={(v) => `${v.toFixed(0)}`} formatTooltip={(v) => `${v.toFixed(1)}/min`} />
        </div>
      )}

      {distRows.length > 0 && topOp !== null && (
        <Card>
          <p className="m-0 text-[13px] font-semibold">Latency distribution · {topOp}</p>
          <p className="m-0 mt-1 text-xs text-muted">Invocations per duration bucket, busiest operation</p>
          <ChartContainer config={{ count: { label: "invocations", color: "#4b9fd5" } }} height={180}>
            <BarChart data={distRows} margin={{ top: 12, right: 8, bottom: 0, left: -8 }}>
              <CartesianGrid vertical={false} stroke="#e6e6e6" />
              <XAxis dataKey="bucket" tickLine={false} axisLine={{ stroke: "#d4d4d4" }} tick={{ fontSize: 10, fill: "#777" }} interval={0} angle={-18} dy={8} height={44} />
              <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "#777" }} allowDecimals={false} width={36} />
              <Tooltip content={<ChartTooltipContent />} cursor={{ fill: "#f3f3f3" }} />
              <Bar dataKey="count" fill="var(--color-count)" radius={[3, 3, 0, 0]} maxBarSize={28} isAnimationActive={!reduced} />
            </BarChart>
          </ChartContainer>
        </Card>
      )}

      {bubblePoints.length > 1 && (
        <Card>
          <p className="m-0 text-[13px] font-semibold">Cost map by operation</p>
          <p className="m-0 mt-1 text-xs text-muted">Horizontal: p90 utilization (cores) · vertical: p90 peak MiB · size: calls</p>
          <CorrelationScatter
            points={bubblePoints}
            xLabel="p90 util (cores)"
            yLabel="p90 peak RSS (MiB)"
            formatX={(v) => v.toFixed(2)}
            formatY={(v) => Math.round(v).toLocaleString("en-US")}
            height={240}
          />
        </Card>
      )}

      {ops.length > 0 && (
        <Card>
          <p className="m-0 text-[13px] font-semibold">Outcomes by operation</p>
          <p className="m-0 mt-1 text-xs text-muted">Cell shade scales with the count in its column</p>
          <div className="mt-3 grid gap-1" style={{ gridTemplateColumns: `minmax(120px,220px) repeat(3,minmax(0,1fr))` }}>
            <span />
            {["success", "gate failed", "other"].map((h) => (
              <span key={h} className="text-center text-[11px] font-semibold uppercase tracking-wide text-muted">{h}</span>
            ))}
            {outcomeRows.map((row) => {
              const cells = [
                { key: "success", value: row.success, color: "0,169,79" },
                { key: "gate_failed", value: row.gate_failed, color: "212,51,63" },
                { key: "other", value: row.other, color: "119,119,119" },
              ];
              return [
                <span key={`${row.operation}-name`} className="truncate font-mono text-xs" title={row.operation}>{row.operation}</span>,
                ...cells.map((c) => (
                  <span
                    key={`${row.operation}-${c.key}`}
                    title={`${row.operation} · ${c.key}: ${fmtInt(c.value)}`}
                    className="rounded px-2 py-1 text-center text-xs tabular-nums"
                    style={{ background: `rgba(${c.color},${(0.06 + 0.5 * (c.value / (heatMax > 0 ? heatMax : 1))).toFixed(2)})` }}
                  >
                    {fmtInt(c.value)}
                  </span>
                )),
              ];
            })}
          </div>
        </Card>
      )}

      {cpuSecLines.some((r) => lineOps.some((op) => r[op] != null)) && (
        <MultiSeriesCard
          title="CPU time p90 over time"
          sub="Seconds of CPU consumed per invocation for the busiest operations · sampled every 5 s"
          rows={cpuSecLines}
          ops={lineOps}
          formatY={(v) => (v < 1 ? `${Math.round(v * 1000)}ms` : `${v.toFixed(1)}s`)}
          formatTooltip={(v) => fmtSecs(v)}
        />
      )}

      {ops.length > 0 && (
        <Card className="overflow-x-auto p-0">
          <table className="w-full border-separate border-spacing-0 text-[13px] tabular-nums">
            <thead>
              <tr>
                {["operation", "calls", "p50", "p90", "p99", "p90 util (cores)", "p90 peak RSS"].map((h) => (
                  <th key={h} className="border-b border-rulestrong bg-[#fafafa] px-3 py-2 text-left text-[11px] font-semibold uppercase tracking-wide text-muted">
                    {h}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {tableRows.map((row) => (
                <tr key={row.operation} className="hover:[&>td]:bg-[#f7fafc]">
                  <td className="max-w-[160px] truncate border-b border-rule px-3 py-[7px] font-mono text-xs" title={row.operation}>{row.operation}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtInt(row.calls)}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtSecs(row.p50)}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtSecs(row.p90)}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtSecs(row.p99)}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{fmtCores(row.cpuP90)}</td>
                  <td className="border-b border-rule px-3 py-[7px] text-right">{row.rssP90 == null ? "—" : `${Math.round(row.rssP90).toLocaleString("en-US")} MiB`}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}

      {findingRows.length > 0 && (
        <Card>
          <p className="m-0 text-[13px] font-semibold">Gate findings by kind and state</p>
          <p className="m-0 mt-1 text-xs text-muted">Cumulative since the store was created · deleting the directory resets counts</p>
          <ChartContainer config={{ value: { label: "findings", color: "#ed7d20" } }} height={Math.min(320, 60 + findingRows.length * 28)}>
            <BarChart data={findingRows} margin={{ top: 12, right: 8, bottom: 0, left: -8 }} layout="vertical">
              <CartesianGrid horizontal={false} stroke="#e6e6e6" />
              <XAxis type="number" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "#777" }} allowDecimals={false} tickFormatter={(v: number) => Math.round(v).toLocaleString("en-US")} />
              <YAxis type="category" dataKey="label" tickLine={false} axisLine={false} tick={{ fontSize: 11, fill: "#333" }} width={axisWidth(findingRows.map((r) => r.label))} />
              <Tooltip content={<ChartTooltipContent />} cursor={{ fill: "#f3f3f3" }} />
              <Bar dataKey="value" fill="var(--color-value)" radius={[0, 3, 3, 0]} maxBarSize={22} isAnimationActive={!reduced} />
            </BarChart>
          </ChartContainer>
        </Card>
      )}

      {findings.length === 0 && (
        <p className="m-0 flex items-center gap-2 text-xs text-muted">
          <AlertTriangle size={14} /> No gate findings recorded yet.
        </p>
      )}
    </div>
  );
});

export function MutationCard({ summary }: { summary: MutationSummary | null }) {
  if (!summary || summary.total == null) return null;
  return (
    <Card>
      <CardTitle>Mutation</CardTitle>
      <div className="mt-3 flex items-center gap-4">
        <Donut
          segments={[
            { label: "killed", value: summary.killed ?? 0, color: "#00a94f" },
            { label: "survived", value: summary.survived ?? 0, color: "#d4333f" },
            { label: "timed out", value: summary.timed_out ?? 0, color: "#ed7d20" },
            { label: "no coverage", value: summary.no_coverage ?? 0, color: "#e6e6e6" },
          ]}
        />
        <div>
          <p className="m-0 text-3xl font-semibold tabular-nums">{summary.score == null ? "—" : summary.score.toFixed(2)}</p>
          <p className="m-0 mt-1 text-xs text-muted">{fmtInt(summary.killed)} killed · {fmtInt(summary.total)} total</p>
        </div>
      </div>
    </Card>
  );
}
