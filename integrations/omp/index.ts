// Native OMP extension for `leadline`. Uses the shared adapter core (same
// core as the Pi extension); the `leadline` binary owns all metrics.
// Post-edit feedback is warn mode only and never gates the agent.

import { registerOmpExtension } from "../agent-adapter-ts/omp/index.js";
import type { ExtensionHost, LeadlineTools } from "../agent-adapter-ts/core/index.js";

export function activate(host: ExtensionHost): LeadlineTools {
  return registerOmpExtension(host, { postEditMode: "warn" });
}
