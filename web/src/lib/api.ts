import type { Project, TelemetrySnapshot } from "./types";

async function get<T>(path: string): Promise<T> {
  const response = await fetch(path, { headers: { accept: "application/json" } });
  if (!response.ok) throw new Error(`GET ${path} → ${response.status}`);
  return (await response.json()) as T;
}

export function fetchProject(): Promise<Project> {
  return get<Project>("/api/project").then(normalizeProject);
}

// The analyzer omits absent optional sections (serde skip_serializing_if),
// so they arrive as undefined rather than null. Normalize once at the
// boundary so every render guard below can rely on null.
function normalizeProject(raw: Project): Project {
  const doc = raw as unknown as Record<string, unknown>;
  return {
    ...(raw as object),
    coverage: (doc["coverage"] as Project["coverage"]) ?? null,
    mutation: (doc["mutation"] as Project["mutation"]) ?? null,
    coupling: (doc["temporal_coupling"] as Project["coupling"]) ?? null,
    snapshots: (doc["snapshots"] as Project["snapshots"]) ?? null,
  } as Project;
}

export function fetchTelemetry(): Promise<TelemetrySnapshot> {
  return get<TelemetrySnapshot>("/api/telemetry");
}

export async function refreshProject(): Promise<void> {
  const response = await fetch("/api/refresh", { method: "POST" });
  if (!response.ok && response.status !== 409) {
    throw new Error(`POST /api/refresh → ${response.status}`);
  }
}
