// Minimal shape of the canonical Project model: only the fields this
// dashboard renders. The server guarantees schema_version 1.

export interface Summary {
  files: number;
  functions: number;
  parse_errors: number;
  dependency_edges: number;
  dependency_cycles: number;
  coverage_percent: number | null;
  mutation_score: number | null;
  duplication_groups: number;
  duplicated_lines: number;
  policy_violations: number;
  risk_model: string | null;
}

export interface ProjectFile {
  path: string;
  language: string | null;
  functions: number | null;
  loc: number | null;
  max_cognitive: number | null;
  max_cyclomatic: number | null;
  max_crap: number | null;
  coverage_percent: number | null;
  parse_errors: number | null;
}

export interface ProjectFunction {
  path: string;
  id: string | null;
  name: string | null;
  start_line: number | null;
  end_line: number | null;
  cognitive: number | null;
  cyclomatic: number | null;
  crap: number | null;
  coverage: number | null;
  max_nesting: number | null;
}

export interface RiskRow {
  path: string | null;
  score: number | null;
  components: Record<string, number | null>;
  max_cognitive: number | null;
  max_cyclomatic: number | null;
  changes: number | null;
  contributors: number | null;
  concentration_percent: number | null;
  blast_radius: number | null;
  fan_in: number | null;
  fan_out: number | null;
  policy_severity: string | null;
}

export interface Violation {
  rule: string | null;
  severity: string | null;
  source: string | null;
  target: string | null;
  status: string | null;
}

export interface TrendPoint {
  commit: string | null;
  commit_timestamp: number | null;
  coverage_percent: number | null;
  max_risk_score: number | null;
  functions: number | null;
  duplicated_lines: number | null;
}

export interface CoverageSection {
  covered_functions: number | null;
  functions: number | null;
  percent: number | null;
}

export interface CouplingEdge {
  source: string | null;
  target: string | null;
  co_changes: number | null;
  directional: number | null;
  reverse_directional: number | null;
  jaccard: number | null;
}

export interface CouplingSection {
  available: boolean;
  reason: string | null;
  edges: CouplingEdge[];
}

export interface MutationSummary {
  total: number | null;
  killed: number | null;
  survived: number | null;
  timed_out: number | null;
  no_coverage: number | null;
  score: number | null;
}

export interface Project {
  meta: {
    schema_version: number;
    metric_profile: string | null;
    analyzer_version: string | null;
    generated_from: string | null;
    head_commit: string | null;
  };
  summary: Summary;
  files: ProjectFile[];
  functions: ProjectFunction[];
  risk: { model: string | null; window: string | null; rows: RiskRow[] };
  coverage: CoverageSection | null;
  coupling: CouplingSection | null;
  mutation: { summary: MutationSummary } | null;
  architecture_violations: Violation[];
  snapshots: TrendPoint[] | null;
}

// Local telemetry store snapshot served by GET /api/telemetry. Mirrors the
// documented store shape: fixed families, closed label sets, no identities.
export interface CounterRow {
  metric: string;
  labels: Record<string, string>;
  value: number;
}

export interface HistogramRow {
  metric: string;
  labels: Record<string, string>;
  count: number;
  sum: number;
  buckets: number[];
  bounds: number[];
}

export interface SeriesOpPoint {
  operation: string;
  latency_p90: number | null;
  cpu_p90: number | null;
  cpu_seconds_p90: number | null;
  rss_p90: number | null;
}

export interface SeriesPoint {
  t_ms: number;
  cpu_mc: number | null;
  rss_bytes: number | null;
  invocations: number;
  findings: number;
  ops: SeriesOpPoint[];
}

export interface TelemetrySnapshot {
  status: "ok" | "disabled";
  reason?: string;
  schema_version?: number;
  counters?: CounterRow[];
  summaries?: HistogramRow[];
  gauges?: CounterRow[];
  series?: SeriesPoint[];
}

export function num(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function str(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

export function fmtInt(value: number | null | undefined): string {
  return value == null ? "—" : Math.round(value).toLocaleString("en-US");
}

export function fmtPct(value: number | null | undefined, digits = 1): string {
  return value == null ? "—" : `${value.toFixed(digits)}%`;
}

export function fmtDec(value: number | null | undefined, digits = 2): string {
  return value == null ? "—" : value.toFixed(digits);
}
