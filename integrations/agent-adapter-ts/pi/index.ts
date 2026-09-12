// Thin Pi registration shim: lifecycle and registration only.
// Tool behavior, binary discovery, and formatting live in ../core/index.js.

import { DEFAULT_BASE, createTools, postEditFeedback } from "../core/index.js";
import type { ChangedInput, ExtensionHost, LeadlineTools, PostEditMode } from "../core/index.js";

export interface PiExtensionOptions {
  postEditMode?: PostEditMode;
}

export function registerPiExtension(host: ExtensionHost, options: PiExtensionOptions): LeadlineTools {
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
