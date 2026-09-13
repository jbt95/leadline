// Native Pi extension entrypoint. Pi loads this file and calls the default
// export with the ExtensionAPI; registration lives in the shared adapter.

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerLeadlineExtension } from "../agent-adapter-ts/pi/index.js";

export default function (pi: ExtensionAPI): void {
  registerLeadlineExtension(pi);
}
