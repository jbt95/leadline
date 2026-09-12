// Native Pi extension for `leadline`. Uses the shared adapter core; the
// `leadline` binary owns all metrics. Post-edit feedback is warn mode only
// and never gates the agent.

import { registerPiExtension } from "../agent-adapter-ts/pi/index.js";
import type { ExtensionHost, LeadlineTools } from "../agent-adapter-ts/core/index.js";

export function activate(host: ExtensionHost): LeadlineTools {
  return registerPiExtension(host, { postEditMode: "warn" });
}
