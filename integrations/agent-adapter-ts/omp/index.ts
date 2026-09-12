// Thin OMP registration shim: lifecycle and registration only.
// Tool behavior, binary discovery, and formatting live in ../core/index.js
// and are shared with the Pi shim. The `leadline mcp` server remains the
// fallback for setups where this native extension is not installed; see
// integrations/omp/README.md.

import { DEFAULT_BASE, createTools, postEditFeedback } from "../core/index.js";
import type { ChangedInput, ExtensionHost, LeadlineTools, PostEditMode } from "../core/index.js";

export interface OmpExtensionOptions {
  postEditMode?: PostEditMode;
}

export function registerOmpExtension(host: ExtensionHost, options: OmpExtensionOptions): LeadlineTools {
  const tools = createTools();
  host.registerTools?.(tools);
  const mode = options.postEditMode ?? "warn";
  host.onPostEdit?.((event) => {
    const input: ChangedInput = { base: event.base ?? DEFAULT_BASE };
    if (event.path !== undefined) {
      input.path = event.path;
    }
    return postEditFeedback(input, mode);
  });
  return tools;
}
