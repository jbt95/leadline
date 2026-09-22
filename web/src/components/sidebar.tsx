import { createContext, useContext, useState, type ReactNode } from "react";
import { Download, PanelLeft, RefreshCw, Search, X } from "lucide-react";
import { cn } from "./ui";
import { ROUTES, hrefFor, navigate } from "../lib/route";
import type { Project } from "../lib/types";

// ── shadcn-style sidebar ─────────────────────────────────────────────
// Vendored following the shadcn sidebar hierarchy (SidebarProvider >
// Sidebar > SidebarHeader/Content/Footer, SidebarGroup > SidebarMenu >
// SidebarMenuItem > SidebarMenuButton, plus SidebarTrigger/Inset/Rail).
// shadcn ships these as copy-paste components; here they are hand-rolled
// in the same shape so the offline bundle gains no new dependency.

interface SidebarState {
  collapsed: boolean;
  toggleCollapsed: () => void;
  drawer: boolean;
  setDrawer: (open: boolean) => void;
}

const SidebarContext = createContext<SidebarState | null>(null);

export function useSidebar(): SidebarState {
  const value = useContext(SidebarContext);
  if (value === null) throw new Error("useSidebar must be used inside SidebarProvider");
  return value;
}

function readCollapsed(): boolean {
  try {
    return localStorage.getItem("leadline:sidebar-collapsed") === "1";
  } catch {
    return false;
  }
}

export function SidebarProvider({ children }: { children: ReactNode }) {
  // Lazy initializer reads storage during render; the toggle writes in its
  // handler. No effect needed for either direction.
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const [drawer, setDrawer] = useState(false);
  const toggleCollapsed = () => {
    setCollapsed((current) => {
      const next = !current;
      try {
        localStorage.setItem("leadline:sidebar-collapsed", next ? "1" : "0");
      } catch {
        // Private mode: the toggle still works for this session.
      }
      return next;
    });
  };
  return (
    <SidebarContext.Provider value={{ collapsed, toggleCollapsed, drawer, setDrawer }}>
      {children}
    </SidebarContext.Provider>
  );
}

export function SidebarHeader({ children }: { children: ReactNode }) {
  return <div className="flex items-center gap-2 border-b border-rule px-4 py-3">{children}</div>;
}

export function SidebarContent({ children }: { children: ReactNode }) {
  return <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">{children}</div>;
}

export function SidebarFooter({ children }: { children: ReactNode }) {
  return <div className="border-t border-rule px-4 py-2.5">{children}</div>;
}

export function SidebarGroup({ children }: { children: ReactNode }) {
  return <div className="flex flex-col gap-0.5">{children}</div>;
}

export function SidebarGroupLabel({ children }: { children: ReactNode }) {
  const { collapsed } = useSidebar();
  return (
    <p className={cn("m-0 px-2 pb-1 pt-1 text-[11px] font-semibold uppercase tracking-wide text-muted", collapsed && "lg:hidden")}>
      {children}
    </p>
  );
}

export function SidebarGroupContent({ children }: { children: ReactNode }) {
  return <div>{children}</div>;
}

export function SidebarMenu({ children }: { children: ReactNode }) {
  return <ul className="m-0 flex list-none flex-col gap-0.5 p-0">{children}</ul>;
}

export function SidebarMenuItem({ children }: { children: ReactNode }) {
  return <li className="m-0 p-0">{children}</li>;
}

export function SidebarMenuButton({ active, href, download, title, onClick, disabled, children }: { active?: boolean; href?: string; download?: string; title?: string; onClick?: () => void; disabled?: boolean; children: ReactNode }) {
  const cls = active
    ? "flex items-center gap-2 rounded border-l-[3px] border-sky bg-[#eef5fb] px-3 py-2 text-[13px] font-semibold text-deep no-underline"
    : "flex items-center gap-2 rounded border-l-[3px] border-transparent px-3 py-2 text-[13px] text-ink no-underline hover:bg-page hover:text-deep";
  if (href === undefined) {
    return (
      <button
        type="button"
        title={title}
        disabled={disabled}
        onClick={onClick}
        className={cn(cls, "w-full cursor-pointer border-0 text-left disabled:cursor-progress disabled:opacity-60")}
      >
        {children}
      </button>
    );
  }
  return (
    <a
      href={href}
      download={download}
      title={title}
      aria-current={active ? "page" : undefined}
      onClick={onClick}
      className={cls}
    >
      {children}
    </a>
  );
}

export function SidebarMenuBadge({ children }: { children: ReactNode }) {
  const { collapsed } = useSidebar();
  return (
    <span className={cn("ml-auto rounded-full bg-blood px-1.5 py-px text-[11px] font-semibold tabular-nums text-white", collapsed && "lg:hidden")}>
      {children}
    </span>
  );
}

/** Thin clickable rail on the outer edge; desktop only. */
export function SidebarRail({ onToggle }: { onToggle: () => void }) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-label="Toggle sidebar width"
      title="Toggle sidebar width"
      className="absolute -right-2 top-16 hidden h-8 w-2 cursor-ew-resize rounded-full border-0 bg-transparent p-0 hover:bg-rule lg:block"
    />
  );
}

/** Header button: collapses the sidebar to icons on desktop. The mobile
    drawer has its own opener in MobileBar and its own close button. */
export function SidebarTrigger({ className }: { className?: string }) {
  const { collapsed, toggleCollapsed } = useSidebar();
  return (
    <button
      type="button"
      onClick={toggleCollapsed}
      aria-label={collapsed ? "Expand report navigation" : "Collapse report navigation"}
      title="Toggle sidebar width"
      className={cn(
        "hidden cursor-pointer items-center rounded border border-rulestrong bg-card p-1.5 text-muted hover:border-sky hover:text-deep lg:inline-flex",
        className,
      )}
    >
      <PanelLeft size={15} />
    </button>
  );
}

/** Sticky drawer opener for mobile: the sidebar is an overlay below lg, so
    the menu button lives here instead of a top bar. */
export function MobileBar({ onPalette }: { onPalette: () => void }) {
  const { setDrawer } = useSidebar();
  return (
    <div className="sticky top-0 z-20 -mx-1 flex items-center gap-2 bg-page py-2 lg:hidden">
      <button
        type="button"
        onClick={() => setDrawer(true)}
        aria-label="Open report navigation"
        className="inline-flex cursor-pointer items-center rounded border border-rulestrong bg-card p-1.5 text-muted hover:border-sky hover:text-deep"
      >
        <PanelLeft size={15} />
      </button>
      <img className="rounded" src="/logo.svg" alt="" width={22} height={22} />
      <span className="text-sm font-bold leading-none">leadline</span>
      <button
        type="button"
        onClick={onPalette}
        aria-label="Jump to a section, file, or function"
        className="ml-auto inline-flex cursor-pointer items-center gap-1 rounded border border-rulestrong bg-card px-2 py-1 text-xs font-semibold text-muted hover:border-sky hover:text-deep"
      >
        <Search size={13} />
        <kbd className="rounded border border-rule bg-page px-1 font-mono text-[10px]">⌘K</kbd>
      </button>
    </div>
  );
}

export function SidebarInset({ children }: { children: ReactNode }) {
  return <div className="min-w-0 flex-1">{children}</div>;
}

function SidebarShell({ labelledBy, children }: { labelledBy: string; children: ReactNode }) {
  const { collapsed, drawer } = useSidebar();
  return (
    <aside
      aria-label={labelledBy}
      className={cn(
        "fixed inset-y-0 left-0 z-40 flex w-[264px] flex-col border-r border-rulestrong bg-card motion-safe:transition-transform motion-safe:duration-200",
        drawer ? "translate-x-0" : "-translate-x-full",
        "lg:sticky lg:top-0 lg:z-auto lg:h-screen lg:shrink-0 lg:translate-x-0",
        collapsed ? "lg:w-[64px]" : "lg:w-[264px]",
      )}
    >
      {children}
    </aside>
  );
}

export function AppSidebar({ project, route, policyErrors, status, refreshing, onRefresh, onPalette }: {
  project: Project | null;
  route: string;
  policyErrors: number;
  status: string;
  refreshing: boolean;
  onRefresh: () => void;
  onPalette: () => void;
}) {
  const { collapsed, toggleCollapsed, drawer, setDrawer } = useSidebar();
  const failed = status === "load failed" || status.startsWith("refresh failed");
  return (
    <>
      {drawer && (
        <div
          aria-hidden="true"
          onClick={() => setDrawer(false)}
          className="fixed inset-0 z-30 bg-black/40 lg:hidden"
        />
      )}
      <SidebarShell labelledBy="Report sections">
        <SidebarHeader>
          <img className="flex-none rounded" src="/logo.svg" alt="" width={24} height={24} />
          <span className={cn("text-[15px] font-bold leading-none", collapsed && "lg:hidden")}>leadline</span>
          <span className="ml-auto flex items-center">
            <button
              type="button"
              onClick={() => setDrawer(false)}
              aria-label="Close report navigation"
              className="cursor-pointer rounded border-0 bg-transparent p-1 text-muted hover:text-deep lg:hidden"
            >
              <X size={16} />
            </button>
            <span className="hidden lg:inline">
              <SidebarTrigger />
            </span>
          </span>
        </SidebarHeader>
        <div className={cn("border-b border-rule px-4 py-2.5", collapsed && "lg:hidden")}>
          <p
            className="m-0 truncate text-[13px] font-semibold leading-snug"
            title={project !== null ? `profile ${project.meta.metric_profile ?? "—"} · schema v${project.meta.schema_version}` : undefined}
          >
            {project?.meta.generated_from ?? "project overview"}
          </p>
          {project?.meta.head_commit != null && (
            <p className="m-0 mt-1 font-mono text-[11px] text-muted" title={project.meta.head_commit}>
              {project.meta.head_commit.slice(0, 10)}
            </p>
          )}
        </div>
        <SidebarContent>
          <SidebarGroup>
            <SidebarGroupLabel>Report</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {ROUTES.map((item) => {
                  const active = item.slug === route;
                  return (
                    <SidebarMenuItem key={item.slug}>
                      <SidebarMenuButton
                        active={active}
                        href={hrefFor(item.slug)}
                        title={collapsed ? item.label : undefined}
                        onClick={() => {
                          setDrawer(false);
                          if (active) navigate(item.slug);
                        }}
                      >
                        <item.icon size={16} className="flex-none text-muted" />
                        <span className={cn("truncate", collapsed && "lg:hidden")}>{item.label}</span>
                        {item.slug === "policy" && policyErrors > 0 && (
                          <SidebarMenuBadge>{policyErrors}</SidebarMenuBadge>
                        )}
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  );
                })}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
          <SidebarGroup>
            <div className="pt-3">
              <SidebarGroupLabel>Session</SidebarGroupLabel>
            </div>
            <SidebarGroupContent>
              <SidebarMenu>
                <SidebarMenuItem>
                  <SidebarMenuButton title={collapsed ? "Jump to a section, file, or function (⌘K)" : "Jump to a section, file, or function"} onClick={onPalette}>
                    <Search size={16} className="flex-none text-muted" />
                    <span className={cn("truncate", collapsed && "lg:hidden")}>Search</span>
                    <kbd className={cn("ml-auto rounded border border-rule bg-page px-1 font-mono text-[10px] text-muted", collapsed && "lg:hidden")}>⌘K</kbd>
                  </SidebarMenuButton>
                </SidebarMenuItem>
                <SidebarMenuItem>
                  <SidebarMenuButton
                    title={collapsed ? "Re-analyze the repository" : undefined}
                    onClick={onRefresh}
                    disabled={refreshing}
                  >
                    <RefreshCw size={16} className={refreshing ? "flex-none animate-spin text-muted" : "flex-none text-muted"} />
                    <span className={cn("truncate", collapsed && "lg:hidden")}>{refreshing ? "Refreshing…" : "Refresh"}</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
                <SidebarMenuItem>
                  <SidebarMenuButton href="/api/project" download="project.json" title={collapsed ? "Download the canonical Project JSON" : undefined}>
                    <Download size={16} className="flex-none text-muted" />
                    <span className={cn("truncate", collapsed && "lg:hidden")}>JSON</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </SidebarContent>
        <SidebarFooter>
          <p
            role="status"
            aria-live="polite"
            className={cn(
              "m-0 text-[11px]",
              collapsed && "lg:hidden",
              failed ? "text-blood" : "text-muted",
            )}
          >
            {status}
          </p>
          <p
            className={cn("m-0 mt-1 truncate text-[11px] text-muted", collapsed && "lg:hidden")}
            title={project !== null ? `profile ${project.meta.metric_profile ?? "—"} · analyzer ${project.meta.analyzer_version ?? "—"}` : undefined}
          >
            {project !== null ? `${project.meta.metric_profile ?? "—"} · ${project.meta.analyzer_version ?? "—"}` : "analyzing…"}
          </p>
        </SidebarFooter>
        <SidebarRail onToggle={toggleCollapsed} />
      </SidebarShell>
    </>
  );
}
