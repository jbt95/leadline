import { useEffect, useState } from "react";
import { Activity, BarChart3, Flame, Gauge, Network, Radio, ShieldCheck, TrendingUp, type LucideIcon } from "lucide-react";
import { slugFromHash } from "./pure.mjs";

export interface RouteDef {
  slug: string;
  label: string;
  title: string;
  source: string;
  icon: LucideIcon;
}

// One route per sidebar item. The slug is the single source of truth for
// the hash, the label, and the section header.
export const ROUTES: RouteDef[] = [
  { slug: "overview", label: "Overview", title: "Overall code", source: "summary · coverage · mutation · policy", icon: Gauge },
  { slug: "measures", label: "Measures", title: "Measures", source: "summary · files", icon: BarChart3 },
  { slug: "complexity", label: "Complexity", title: "Complexity distribution", source: "functions[]", icon: Activity },
  { slug: "hotspots", label: "Hotspots", title: "Hotspots", source: "risk.rows", icon: Flame },
  { slug: "coupling", label: "Coupling", title: "Coupling", source: "temporal_coupling", icon: Network },
  { slug: "trends", label: "Trends", title: "Trends", source: "snapshots", icon: TrendingUp },
  { slug: "telemetry", label: "Telemetry", title: "Telemetry", source: "LEADLINE_METRICS_DIR store", icon: Radio },
  { slug: "policy", label: "Policy", title: "Policy violations", source: "architecture_violations", icon: ShieldCheck },
];

export const FALLBACK_ROUTE = "overview";

/** Reads `#/slug`; anything else falls back to the default route. */
export function parseHash(hash: string = window.location.hash): string {
  return slugFromHash(hash, ROUTES.map((r) => r.slug), FALLBACK_ROUTE);
}

export function hrefFor(slug: string): string {
  return `#/${slug}`;
}

/** Hash routes: one route renders at a time, no router dependency, and the
    URL stays deep-linkable in the offline embedded page. The active sidebar
    item derives from this during render, so no scrollspy is needed. */
export function useRoute(): string {
  const [route, setRoute] = useState(() => parseHash());
  useEffect(() => {
    const onChange = () => {
      setRoute(parseHash());
      window.scrollTo({ top: 0 });
    };
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return route;
}

/** Sidebar links use native anchors (copyable, middle-clickable); this only
    covers re-clicking the active route, where no hashchange fires. */
export function navigate(slug: string): void {
  if (parseHash() === slug) window.scrollTo({ top: 0 });
  else window.location.hash = hrefFor(slug);
}
