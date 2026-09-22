import { useEffect, useMemo, useRef, useState } from "react";
import { CornerDownLeft, FileCode2, Flame, FunctionSquare, Search } from "lucide-react";
import type { Project } from "../lib/types";
import { fmtInt } from "../lib/types";

export interface PaletteSection {
  href: string;
  label: string;
}

interface Entry {
  kind: "section" | "file" | "function" | "hotspot";
  label: string;
  sub: string;
  href: string;
  path: string | null;
  fnName: string | null;
}

function rank(query: string, label: string): number {
  const q = query.toLowerCase();
  const l = label.toLowerCase();
  if (l === q) return 0;
  if (l.startsWith(q)) return 1;
  if (l.includes(q)) return 2;
  return -1;
}

export function usePaletteEntries(project: Project | null, sections: PaletteSection[]): Entry[] {
  return useMemo(() => {
    if (project === null) return [];
    const entries: Entry[] = sections.map((s) => ({ kind: "section", label: s.label, sub: "section", href: s.href, path: null, fnName: null }));
    const riskByPath = new Map(project.risk.rows.map((r) => [r.path ?? "", r.score ?? 0]));
    const topFiles = [...project.files]
      .sort((a, b) => (b.loc ?? 0) - (a.loc ?? 0))
      .slice(0, 200);
    for (const f of topFiles) {
      if (f.path == null) continue;
      entries.push({
        kind: riskByPath.has(f.path) ? "hotspot" : "file",
        label: f.path.split("/").pop() ?? f.path,
        sub: `${f.path} · ${fmtInt(f.functions)} fns · ${fmtInt(f.loc)} loc`,
        href: "#/hotspots",
        path: f.path,
        fnName: null,
      });
    }
    const topFns = [...project.functions]
      .sort((a, b) => (b.cognitive ?? 0) - (a.cognitive ?? 0))
      .slice(0, 200);
    for (const fn of topFns) {
      if (fn.name == null) continue;
      entries.push({
        kind: "function",
        label: `${fn.name}()`,
        sub: `${fn.path} · cog ${fn.cognitive ?? "—"} · cyc ${fn.cyclomatic ?? "—"}`,
        href: "#/complexity",
        path: fn.path,
        fnName: fn.name,
      });
    }
    return entries;
  }, [project, sections]);
}

const KIND_ICON = { section: Search, file: FileCode2, function: FunctionSquare, hotspot: Flame } as const;

export function Inspector({ project, path, fnName, onClose, onJump }: {
  project: Project;
  path: string | null;
  fnName: string | null;
  onClose: () => void;
  onJump: (href: string) => void;
}) {
  const file = path != null ? project.files.find((f) => f.path === path) ?? null : null;
  const risk = path != null ? project.risk.rows.find((r) => r.path === path) ?? null : null;
  const fn = fnName != null ? project.functions.find((f) => f.name === fnName && (path == null || f.path === path)) ?? null : null;
  const rows: Array<[string, string]> = [];
  if (fn !== null) {
    rows.push(["function", `${fn.name}()`,]);
    rows.push(["location", `${fn.path}:${fn.start_line ?? "?"}–${fn.end_line ?? "?"}`]);
    rows.push(["cyclomatic / cognitive", `${fn.cyclomatic ?? "—"} / ${fn.cognitive ?? "—"}`]);
    rows.push(["crap / coverage", `${fn.crap ?? "—"} / ${fn.coverage ?? "—"}`]);
  } else if (file !== null) {
    rows.push(["language", file.language ?? "—"]);
    rows.push(["functions / loc", `${fmtInt(file.functions)} / ${fmtInt(file.loc)}`]);
    rows.push(["max cognitive / cyclomatic", `${file.max_cognitive ?? "—"} / ${file.max_cyclomatic ?? "—"}`]);
    rows.push(["coverage", file.coverage_percent == null ? "—" : `${file.coverage_percent.toFixed(1)}%`]);
  }
  if (risk !== null) rows.push(["risk score", (risk.score ?? 0).toFixed(2)]);
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 p-4 pt-24" onClick={onClose} role="presentation">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={fn?.name ?? file?.path ?? "details"}
        className="w-full max-w-lg overflow-hidden rounded border border-rulestrong bg-card shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="border-b border-rule px-4 py-3">
          <p className="m-0 font-mono text-sm font-semibold">{fn !== null ? `${fn.name}()` : (file?.path ?? path ?? "")}</p>
          {file !== null && <p className="m-0 mt-0.5 font-mono text-xs text-muted">{file.path}</p>}
        </div>
        <dl className="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 px-4 py-3 text-[13px]">
          {rows.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="text-muted">{k}</dt>
              <dd className="m-0 truncate font-mono text-xs" title={v}>{v}</dd>
            </div>
          ))}
        </dl>
        <div className="flex justify-end gap-2 border-t border-rule px-4 py-2.5">
          <button
            type="button"
            onClick={() => { onJump(fn !== null ? "#/complexity" : "#/hotspots"); onClose(); }}
            className="cursor-pointer rounded border border-rulestrong bg-card px-2.5 py-1 text-xs font-semibold hover:border-sky hover:text-deep"
          >
            {fn !== null ? "Open complexity" : "Open hotspots"}
          </button>
          <button
            type="button"
            onClick={onClose}
            className="cursor-pointer rounded border border-sky bg-sky px-2.5 py-1 text-xs font-semibold text-white hover:bg-deep"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

export function CommandPalette({ project, sections, open, onClose, onInspect }: {
  project: Project | null;
  sections: PaletteSection[];
  open: boolean;
  onClose: () => void;
  onInspect: (path: string | null, fnName: string | null) => void;
}) {
  const entries = usePaletteEntries(project, sections);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const results = useMemo(() => {
    const q = query.trim();
    if (q === "") return entries.filter((e) => e.kind === "section");
    return entries
      .map((e) => ({ e, r: Math.min(rank(q, e.label), rank(q, e.sub)) }))
      .filter((x) => x.r >= 0)
      .sort((a, b) => a.r - b.r)
      .slice(0, 12)
      .map((x) => x.e);
  }, [entries, query]);

  // Reset when opened: palette state tracks the open prop, so adjust during
  // render with a previous-open guard. Only input focus stays an effect,
  // because focusing the browser's input is an external DOM sync.
  const [prevOpen, setPrevOpen] = useState(open);
  if (open !== prevOpen) {
    setPrevOpen(open);
    if (open) {
      setQuery("");
      setCursor(0);
    }
  }

  useEffect(() => {
    if (open) requestAnimationFrame(() => inputRef.current?.focus());
  }, [open ]);

  if (!open) return null;
  const jump = (entry: Entry) => {
    onClose();
    if (entry.kind === "file" || entry.kind === "function") {
      onInspect(entry.path, entry.fnName);
      return;
    }
    // Section entries carry hash routes ("#/hotspots"); assigning the hash
    // navigates the sidebar route, which scrolls to top on change.
    window.location.hash = entry.href;
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 p-4 pt-24" onClick={onClose} role="presentation">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Jump to section, file, or function"
        className="w-full max-w-lg overflow-hidden rounded border border-rulestrong bg-card shadow-lg"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          else if (e.key === "ArrowDown") { e.preventDefault(); setCursor((c) => Math.min(results.length - 1, c + 1)); }
          else if (e.key === "ArrowUp") { e.preventDefault(); setCursor((c) => Math.max(0, c - 1)); }
          else if (e.key === "Enter" && results[cursor] !== undefined) jump(results[cursor]);
        }}
      >
        <div className="flex items-center gap-2 border-b border-rule px-3">
          <Search size={15} className="flex-none text-muted" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
            setQuery(e.target.value);
            setCursor(0);
          }}
            placeholder="Jump to a section, file, or function…"
            aria-label="Jump to a section, file, or function"
            className="w-full border-0 bg-transparent py-2.5 text-sm outline-none placeholder:text-muted"
          />
        </div>
        <ul role="listbox" aria-label="results" className="m-0 max-h-[320px] list-none overflow-y-auto p-1">
          {results.length === 0 && (
            <li className="px-3 py-4 text-center text-xs text-muted">No matches — try a filename, function, or section.</li>
          )}
          {results.map((entry, i) => {
            const Icon = KIND_ICON[entry.kind];
            return (
              <li
                key={`${entry.kind}-${entry.label}-${i}`}
                role="option"
                aria-selected={i === cursor}
                onMouseMove={() => setCursor(i)}
                onClick={() => jump(entry)}
                className={i === cursor ? "flex cursor-pointer items-center gap-2.5 rounded bg-[#eef5fb] px-3 py-2" : "flex cursor-pointer items-center gap-2.5 rounded px-3 py-2 hover:bg-page"}
              >
                <Icon size={14} className="flex-none text-muted" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-[13px]">{entry.label}</span>
                  <span className="block truncate text-xs text-muted">{entry.sub}</span>
                </span>
                {i === cursor && <CornerDownLeft size={13} className="flex-none text-muted" />}
              </li>
            );
          })}
        </ul>
        <p className="m-0 border-t border-rule px-3 py-1.5 text-[11px] text-muted">↑↓ navigate · ↵ open · esc close · sections, top files, top functions</p>
      </div>
    </div>
  );
}
