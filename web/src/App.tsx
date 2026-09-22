import { useCallback, useEffect, useState } from "react";
import { AlertTriangle, FileWarning } from "lucide-react";
import { Complexity, Coupling, Gate, Hotspots, Measures, MutationCard, Overview, Telemetry, Trends, Violations } from "./components/sections";
import { AppSidebar, MobileBar, SidebarInset, SidebarProvider, useSidebar } from "./components/sidebar";
import { CommandPalette, Inspector } from "./components/palette";
import { fetchProject, fetchTelemetry, refreshProject } from "./lib/api";
import { ROUTES, useRoute } from "./lib/route";
import type { Project, TelemetrySnapshot } from "./lib/types";

// The stats page is routed: the sidebar renders one route at a time from the
// location hash (#/overview … #/policy), so every item is a deep-linkable
// route and the active item derives from the route during render. The sidebar
// fills the whole viewport height and owns the former top bar: brand and
// project meta on top, a Session group for search/refresh/download, and the
// refresh status in the footer.

function RouteHeader({ slug }: { slug: string }) {
  const def = ROUTES.find((r) => r.slug === slug) ?? ROUTES[0];
  return (
    <div className="mb-3 flex items-baseline justify-between gap-4">
      <h2 className="m-0 text-lg font-semibold">{def.title}</h2>
      <p className="m-0 text-right font-mono text-xs text-muted">{def.source}</p>
    </div>
  );
}

/** Global keys: ⌘K/Ctrl+K toggles the palette, Escape closes the drawer and
    overlays. A keyboard subscription syncs with the browser, so it stays an
    effect; everything it triggers lives in handlers. */
function Shortcuts({ onPalette, onEscape }: { onPalette: () => void; onEscape: () => void }) {
  const { setDrawer } = useSidebar();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        onPalette();
      } else if (e.key === "Escape") {
        setDrawer(false);
        onEscape();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onPalette, onEscape, setDrawer]);
  return null;
}

export default function App() {
  const route = useRoute();
  const [project, setProject] = useState<Project | null>(null);
  const [telemetry, setTelemetry] = useState<TelemetrySnapshot | null>(null);
  const [telemetryError, setTelemetryError] = useState<string | null>(null);
  const [status, setStatus] = useState("analyzing…");
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [inspected, setInspected] = useState<{ path: string | null; fnName: string | null } | null>(null);
  const [live, setLive] = useState(true);

  const loadTelemetry = useCallback(async () => {
    try {
      setTelemetry(await fetchTelemetry());
      setTelemetryError(null);
    } catch (err) {
      // Telemetry is advisory: a failed fetch must never fail the page.
      setTelemetryError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const load = useCallback(async () => {
    try {
      const doc = await fetchProject();
      if (doc.meta.schema_version !== 1) {
        throw new Error(`unsupported schema_version ${doc.meta.schema_version}: this page renders version 1`);
      }
      const apply = () => {
        setProject(doc);
        setError(null);
        setUpdatedAt(new Date().toLocaleTimeString());
        setStatus("ready · refresh re-analyzes the repository");
      };
      // Morph between data states where supported; plain swap elsewhere.
      if ("startViewTransition" in document) document.startViewTransition(apply);
      else apply();
    } catch (err) {
      setError(`Cannot load the project model: ${err instanceof Error ? err.message : String(err)}. Fetch /api/project directly, or restart leadline stats.`);
      setStatus("load failed");
    }
    await loadTelemetry();
  }, [loadTelemetry]);

  // Grafana-style live tail for telemetry only: cheap endpoint, paused when
  // the tab hides. Project data refreshes on demand, never on a timer.
  useEffect(() => {
    if (!live || project === null) return;
    const id = window.setInterval(() => {
      if (!document.hidden) void loadTelemetry();
    }, 15000);
    return () => window.clearInterval(id);
  }, [live, project, loadTelemetry]);

  useEffect(() => {
    void load();
  }, [load]);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    setStatus("refreshing · re-analyzing the repository");
    try {
      await refreshProject();
      await load();
      setStatus("refresh complete");
    } catch (err) {
      setStatus(`refresh failed: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setRefreshing(false);
    }
  }, [load]);

  const togglePalette = useCallback(() => setPaletteOpen((v) => !v), []);
  const openPalette = useCallback(() => setPaletteOpen(true), []);
  const closeOverlays = useCallback(() => {
    setPaletteOpen(false);
    setInspected(null);
  }, []);

  const policyErrors = project?.architecture_violations.filter(
    (v) => (v.severity ?? "").toLowerCase() === "error",
  ).length ?? 0;

  return (
    <SidebarProvider>
      <div className="flex min-h-screen items-start">
        <AppSidebar
          project={project}
          route={route}
          policyErrors={policyErrors}
          status={status}
          refreshing={refreshing}
          onRefresh={() => void onRefresh()}
          onPalette={openPalette}
        />
        <SidebarInset>
          <div className="mx-auto min-h-screen max-w-[1200px] px-6 pb-12 pt-2 max-[1100px]:px-4 lg:pt-4">
            <MobileBar onPalette={openPalette} />
            <main className="min-w-0">
              {error !== null && (
                <p className="m-0 mb-4 flex items-center gap-2 rounded border border-[#f2b8bd] bg-bloodbg px-4 py-3 text-[13px] text-blood">
                  <FileWarning size={16} /> {error}
                </p>
              )}
              {project === null && error === null ? (
                <div aria-label="Loading" className="flex flex-col gap-4">
                  {[0, 1, 2].map((i) => (
                    <div key={i} className="rounded border border-rule bg-card p-4">
                      <div className="h-4 w-40 animate-pulse rounded bg-rule" />
                      <div className="mt-3 h-24 animate-pulse rounded bg-page" />
                    </div>
                  ))}
                </div>
              ) : (
                project !== null && (
                  <>
                    {/* Keyed by readout time so the verdict-settle animation replays
                        exactly when a new verdict arrives — load or refresh. */}
                    <div aria-label="Quality gate" className="mb-6" key={updatedAt ?? "loading"}>
                      <Gate project={project} />
                    </div>
                    {route === "overview" && (
                      <>
                        <RouteHeader slug="overview" />
                        <Overview project={project} />
                        {project.mutation !== null && (
                          <div className="mt-4 grid grid-cols-[repeat(auto-fit,minmax(min(300px,100%),1fr))] gap-4">
                            <MutationCard summary={project.mutation.summary} />
                          </div>
                        )}
                      </>
                    )}
                    {route === "measures" && (
                      <>
                        <RouteHeader slug="measures" />
                        <Measures project={project} />
                      </>
                    )}
                    {route === "complexity" && (
                      <>
                        <RouteHeader slug="complexity" />
                        <Complexity project={project} />
                      </>
                    )}
                    {route === "hotspots" && (
                      <>
                        <RouteHeader slug="hotspots" />
                        <Hotspots rows={project.risk.rows} files={project.files} />
                      </>
                    )}
                    {route === "coupling" && (
                      <>
                        <RouteHeader slug="coupling" />
                        <Coupling coupling={project.coupling} />
                      </>
                    )}
                    {route === "trends" && (
                      <>
                        <RouteHeader slug="trends" />
                        <Trends project={project} />
                      </>
                    )}
                    {route === "telemetry" && (
                      <>
                        <RouteHeader slug="telemetry" />
                        <div className="mb-3 flex items-center gap-2">
                          <span className="flex items-center gap-1.5 text-xs text-muted" aria-live="polite">
                            <span className={live ? "live-dot inline-block h-2 w-2 rounded-full bg-leaf" : "inline-block h-2 w-2 rounded-full bg-rule"} />
                            {live ? "Live · refreshes every 15 s" : "Paused"}
                          </span>
                          <button
                            type="button"
                            onClick={() => setLive((v) => !v)}
                            className="cursor-pointer rounded border border-rulestrong bg-card px-2 py-0.5 text-[11px] font-semibold text-muted hover:border-sky hover:text-deep"
                          >
                            {live ? "Pause" : "Resume"}
                          </button>
                        </div>
                        <Telemetry snapshot={telemetry} error={telemetryError} onRetry={loadTelemetry} />
                      </>
                    )}
                    {route === "policy" && (
                      <>
                        <RouteHeader slug="policy" />
                        <Violations rows={project.architecture_violations} />
                      </>
                    )}
                    {project.summary.parse_errors > 0 && (
                      <p className="m-0 mt-4 flex items-center gap-2 text-xs text-muted">
                        <AlertTriangle size={14} /> {project.summary.parse_errors} parse errors — see the quality gate above.
                      </p>
                    )}
                    {updatedAt !== null && (
                      <p className="m-0 mt-2 text-xs text-muted">Last updated {updatedAt} · refresh re-analyzes the repository.</p>
                    )}
                  </>
                )
              )}
            </main>
          </div>
        </SidebarInset>
      </div>
      <Shortcuts onPalette={togglePalette} onEscape={closeOverlays} />
      <CommandPalette
        project={project}
        sections={ROUTES.map((item) => ({ href: `#/${item.slug}`, label: item.label }))}
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        onInspect={(path, fnName) => setInspected({ path, fnName })}
      />
      {inspected !== null && project !== null && (
        <Inspector
          project={project}
          path={inspected.path}
          fnName={inspected.fnName}
          onClose={() => setInspected(null)}
          onJump={(href) => {
            window.location.hash = href;
          }}
        />
      )}
    </SidebarProvider>
  );
}
