// shadcn-style charts over Recharts: donut, histogram, and trend area.
// Pure renderers of analyzer values; nulls break the trend line.

import {
  Area,
  Bar,
  CartesianGrid,
  Cell,
  Pie,
  PieChart,
  BarChart,
  AreaChart,
  ResponsiveContainer,
  Scatter,
  ScatterChart,
  Sankey,
  Treemap,
  Tooltip,
  XAxis,
  YAxis,
  ZAxis,
} from "recharts";
import { useState, type CSSProperties, type ReactElement } from "react";
import { cn } from "./ui";

// shadcn-style chart primitives over Recharts: ChartContainer injects the
// series colors as CSS vars, ChartTooltipContent renders the floating card.
export type ChartConfig = Record<string, { label: string; color: string }>;

function varStyle(config: ChartConfig): CSSProperties {
  const style: Record<string, string> = {};
  for (const [key, entry] of Object.entries(config)) {
    style[`--color-${key}`] = entry.color;
  }
  return style as CSSProperties;
}

export function ChartContainer({
  config,
  height,
  className,
  children,
}: {
  config: ChartConfig;
  height: number | string;
  className?: string;
  children: ReactElement;
}) {
  return (
    <div className={cn("w-full", className)} style={{ ...varStyle(config), height }}>
      <ResponsiveContainer width="100%" height="100%">
        {children}
      </ResponsiveContainer>
    </div>
  );
}

export function ChartTooltipContent({
  active,
  payload,
  label,
  formatter,
}: {
  active?: boolean;
  payload?: Array<{ name?: string; value?: number | string; color?: string; payload?: Record<string, unknown> }>;
  label?: string | number;
  formatter?: (value: number | string, name: string) => string;
}) {
  if (!active || !payload || payload.length === 0) return null;
  return (
    <div className="rounded border border-rule bg-card px-3 py-2 text-xs shadow-md">
      {label !== undefined && <p className="m-0 mb-1 font-semibold">{label}</p>}
      {payload.map((entry, i) => (
        <p key={i} className="m-0 flex items-center gap-2 tabular-nums">
          <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: entry.color }} />
          {entry.name}: {formatter !== undefined ? formatter(entry.value ?? "", String(entry.name)) : entry.value}
        </p>
      ))}
    </div>
  );
}

export function ChartLegendContent({
  payload,
}: {
  payload?: Array<{ value?: string; color?: string }>;
}) {
  if (!payload || payload.length === 0) return null;
  return (
    <ul className="m-0 flex list-none flex-wrap gap-1 gap-x-3 p-0 text-xs text-muted">
      {payload.map((entry, i) => (
        <li key={i} className="flex items-center">
          <i className="mr-1.5 inline-block h-2.5 w-2.5 rounded-full" style={{ background: entry.color }} />
          {entry.value}
        </li>
      ))}
    </ul>
  );
}

export interface Slice {
  label: string;
  value: number;
  color: string;
}

export function Donut({ segments, size = 120 }: { segments: Slice[]; size?: number }) {
  const reduced = useReducedMotion();
  const total = segments.reduce((sum, s) => sum + s.value, 0);
  const data = segments.filter((s) => s.value > 0);
  if (!(total > 0) || data.length === 0) return null;
  const config: ChartConfig = Object.fromEntries(data.map((s) => [s.label, { label: s.label, color: s.color }]));
  return (
    <ChartContainer config={config} height={size}>
      <PieChart>
        <Tooltip content={<ChartTooltipContent />} />
        <Pie data={data} dataKey="value" nameKey="label" innerRadius={size * 0.28} outerRadius={size * 0.38} paddingAngle={1} strokeWidth={0} isAnimationActive={!reduced}>
          {data.map((s) => (
            <Cell key={s.label} fill={s.color} />
          ))}
        </Pie>
      </PieChart>
    </ChartContainer>
  );
}

export function Histogram({ values, height = 180 }: { values: number[]; height?: number }) {
  const reduced = useReducedMotion();
  const buckets = 10;
  const data = Array.from({ length: buckets }, () => 0);
  if (values.length > 0) {
    const low = Math.min(...values);
    const high = Math.max(...values);
    const span = high - low || 1;
    for (const v of values) {
      const i = Math.min(buckets - 1, Math.floor(((v - low) / span) * buckets));
      data[i] += 1;
    }
  }
  const rows = data.map((count, i) => ({ bucket: `b${i + 1}`, count }));
  return (
    <ChartContainer config={{ count: { label: "functions", color: "#4b9fd5" } }} height={height}>
      <BarChart data={rows} margin={{ top: 12, right: 8, bottom: 0, left: -8 }}>
        <CartesianGrid vertical={false} stroke="#e6e6e6" />
        <XAxis dataKey="bucket" tickLine={false} axisLine={{ stroke: "#d4d4d4" }} tick={{ fontSize: 10, fill: "#777" }} interval={1} />
        <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "#777" }} allowDecimals={false} width={36} tickFormatter={(v: number) => Math.round(v).toLocaleString("en-US")} />
        <Tooltip content={<ChartTooltipContent />} cursor={{ fill: "#f3f3f3" }} />
        <Bar dataKey="count" fill="var(--color-count)" radius={[3, 3, 0, 0]} maxBarSize={28} isAnimationActive={!reduced} />
      </BarChart>
    </ChartContainer>
  );
}

export interface TrendDatum {
  label: string;
  value: number | null;
  shown: string;
  t?: number;
}

export function TrendLine({ points, height = 170, formatY, formatTooltip }: { points: TrendDatum[]; height?: number; formatY?: (v: number) => string; formatTooltip?: (v: number) => string }) {
  const reduced = useReducedMotion();
  const valid = points.filter((p) => p.value !== null);
  if (valid.length < 2) return <p className="m-0 text-xs text-muted">Not enough points to draw a line.</p>;
  const fmtAxis = formatY ?? ((v: number) => (v >= 100 ? Math.round(v).toLocaleString("en-US") : v.toFixed(1)));
  const fmtTip = formatTooltip ?? ((v: number) => `${v}`);
  const rows = points.map((p) => ({ label: p.label, value: p.value, shown: p.shown }));
  return (
    <ChartContainer config={{ value: { label: "value", color: "#1d75b3" } }} height={height}>
      <AreaChart data={rows} margin={{ top: 12, right: 8, bottom: 0, left: 0 }}>
        <CartesianGrid vertical={false} stroke="#e6e6e6" />
        <XAxis dataKey="label" tickLine={false} axisLine={{ stroke: "#d4d4d4" }} tick={{ fontSize: 10, fill: "#777" }} minTickGap={24} />
        <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "#777" }} tickFormatter={fmtAxis} width={44} domain={["auto", "auto"]} />
        <Tooltip content={<ChartTooltipContent formatter={(v) => (typeof v === "number" ? fmtTip(v) : `${v}`)} />} cursor={{ stroke: "#d4d4d4" }} />
        <Area type="monotone" dataKey="value" isAnimationActive={!reduced} stroke="var(--color-value)" fill="var(--color-value)" fillOpacity={0.15} strokeWidth={2} connectNulls={false} dot={{ r: 3, fill: "var(--color-value)" }} activeDot={{ r: 4 }} />
      </AreaChart>
    </ChartContainer>
  );
}

export function ChartLegend({ config }: { config: ChartConfig }) {
  return <ChartLegendContent payload={Object.entries(config).map(([, entry]) => ({ value: entry.label, color: entry.color }))} />;
}

/** Recharts animations off when the visitor prefers reduced motion. */
export function useReducedMotion(): boolean {
  const [reduced] = useState(
    () => typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  return reduced;
}

/** Y-axis width that fits the longest category label instead of clipping it. */
export function axisWidth(labels: string[]): number {
  const longest = labels.reduce((m, l) => Math.max(m, l.length), 0);
  return Math.min(220, Math.max(80, longest * 7 + 12));
}
export const LANGUAGE_PALETTE = ["#4b9fd5", "#ed7d20", "#00a94f", "#8172b3", "#eabe06", "#d4333f", "#777777"];

export function paletteColor(index: number): string {
  return LANGUAGE_PALETTE[index % LANGUAGE_PALETTE.length];
}

/** Sequential white-to-red for ordered risk scores 0–100. */
export function riskColor(score: number): string {
  const f = Math.max(0, Math.min(1, score / 100));
  const r = Math.round(230 + (212 - 230) * f);
  const g = Math.round(230 + (51 - 230) * f);
  const b = Math.round(230 + (63 - 230) * f);
  return `rgb(${r},${g},${b})`;
}

export interface ScatterPoint {
  x: number;
  y: number;
  z: number;
  name: string;
  detail: string;
  color: string;
}

/** Correlation scatter: size encodes the third variable, color the category. */
export function CorrelationScatter({ points, height = 260, xLabel, yLabel, formatX, formatY }: { points: ScatterPoint[]; height?: number; xLabel: string; yLabel: string; formatX?: (v: number) => string; formatY?: (v: number) => string }) {
  const reduced = useReducedMotion();
  if (points.length === 0) return <p className="m-0 text-xs text-muted">No points with both values.</p>;
  const fmtX = formatX ?? ((v: number) => `${v}`);
  const fmtY = formatY ?? ((v: number) => `${v}`);
  return (
    <ChartContainer config={{ points: { label: "functions", color: "#4b9fd5" } }} height={height}>
      <ScatterChart margin={{ top: 12, right: 12, bottom: 0, left: 0 }}>
        <CartesianGrid stroke="#e6e6e6" />
        <XAxis type="number" dataKey="x" name={xLabel} tickLine={false} axisLine={{ stroke: "#d4d4d4" }} tick={{ fontSize: 10, fill: "#777" }} tickFormatter={fmtX} label={{ value: xLabel, position: "insideBottomRight", offset: -2, fontSize: 11, fill: "#777" }} />
        <YAxis type="number" dataKey="y" name={yLabel} tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "#777" }} tickFormatter={fmtY} width={56} label={{ value: yLabel, angle: -90, position: "insideLeft", fontSize: 11, fill: "#777" }} />
        <ZAxis type="number" dataKey="z" range={[16, 160]} />
        <Tooltip
          content={({ active, payload }) => {
            if (!active || !payload || payload.length === 0) return null;
            const d = payload[0]?.payload as ScatterPoint | undefined;
            if (!d) return null;
            return (
              <div className="rounded border border-rule bg-card px-3 py-2 text-xs shadow-md">
                <p className="m-0 mb-1 font-mono font-semibold">{d.name}</p>
                <p className="m-0 tabular-nums">{xLabel} {fmtX(d.x)} · {yLabel} {fmtY(d.y)}</p>
                <p className="m-0 mt-1 text-muted">{d.detail}</p>
              </div>
            );
          }}
          cursor={{ strokeDasharray: "3 3" }}
        />
        <Scatter data={points} isAnimationActive={!reduced}>
          {points.map((p, i) => (
            <Cell key={i} fill={p.color} fillOpacity={0.75} />
          ))}
        </Scatter>
      </ScatterChart>
    </ChartContainer>
  );
}

export interface TreemapNode {
  name: string;
  size: number;
  fill: string;
  detail: string;
}

/** Hierarchy treemap: area encodes size, fill encodes the ordered value. */
export function RiskTreemap({ nodes, height = 260 }: { nodes: TreemapNode[]; height?: number }) {
  const reduced = useReducedMotion();
  if (nodes.length === 0) return <p className="m-0 text-xs text-muted">No scored files.</p>;
  return (
    <ChartContainer config={{ files: { label: "files", color: "#d4333f" } }} height={height}>
      <Treemap
        data={nodes}
        dataKey="size"
        nameKey="name"
        stroke="#ffffff"
        isAnimationActive={!reduced}
        content={<TreemapContent />}
      />
    </ChartContainer>
  );
}

function TreemapContent(props: {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  name?: string;
  fill?: string;
  detail?: string;
}) {
  const { x = 0, y = 0, width = 0, height = 0, name = "", fill = "#e6e6e6", detail = "" } = props;
  if (width <= 0 || height <= 0) return null;
  const maxChars = Math.max(0, Math.floor((width - 10) / 6));
  if (maxChars < 2 || height < 20) {
    return (
      <g>
        <rect x={x} y={y} width={width} height={height} fill={fill} stroke="#ffffff">
          <title>{`${name} · ${detail}`}</title>
        </rect>
      </g>
    );
  }
  const shown = name.length > maxChars ? `${name.slice(0, Math.max(0, maxChars - 1))}…` : name;
  return (
    <g>
      <rect x={x} y={y} width={width} height={height} fill={fill} stroke="#ffffff">
        <title>{`${name} · ${detail}`}</title>
      </rect>
      <text x={x + 5} y={y + 15} fontSize={10} fill="#333">
        {shown}
      </text>
    </g>
  );
}

export interface SankeyLink {
  source: string;
  target: string;
  value: number;
}

/** Co-change flow: the top file pairs that change together. */
export function CouplingSankey({ links, height = 280 }: { links: SankeyLink[]; height?: number }) {
  if (links.length === 0) return <p className="m-0 text-xs text-muted">No file pair passed the co-change threshold.</p>;
  const names = [...new Set(links.flatMap((l) => [l.source, l.target]))];
  const index = new Map(names.map((n, i) => [n, i]));
  const data = {
    nodes: names.map((name) => ({ name })),
    links: links.map((l) => ({ source: index.get(l.source) ?? 0, target: index.get(l.target) ?? 0, value: l.value })),
  };
  return (
    <ChartContainer config={{ flow: { label: "co-changes", color: "#4b9fd5" } }} height={height}>
      <Sankey
        data={data}
        nodePadding={12}
        nodeWidth={10}
        link={{ stroke: "#4b9fd5", strokeOpacity: 0.4 }}
        node={{ fill: "#1d75b3" }}
      >
        <Tooltip content={<ChartTooltipContent />} />
      </Sankey>
    </ChartContainer>
  );
}
