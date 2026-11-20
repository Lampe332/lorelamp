// The very first start: no model set up, no character imported. The main suite cannot show it (its
// global setup imports cards and needs a running model), so this config runs `home-first-run.bare.ts`
// alone against a test server with an EMPTY data folder and no model:
//   <temp>/rp-server.exe --port <n> --manage-engine --llama http://127.0.0.1:1 --web-dir <temp>/dist --data-dir <empty folder>
// (no engine.json in it; `--llama` points nowhere so a model server on the default port is not taken for
// this server's model; RP_LMSTUDIO_DIR and RP_ST_SEARCH_ROOTS pointing at empty folders, RP_HF_BASE set
// as for the main suite), then: npx playwright test -c e2e/first-run.config.ts
// with RP_E2E_URL / RP_E2E_TOKEN of that server. The tests import nothing and leave the folder empty.
// engine-setup.bare.ts needs more (RP_GITHUB_API, RP_E2E_DATA, RP_E2E_LLAMA_CPU; see its head) and
// skips itself without. tutorial.spec.ts (the welcome tour, Q9) runs here too: only an empty data
// folder shows the tour by itself. The .bare tests put the tour aside while they run.

import { defineConfig } from "@playwright/test";
import base from "../playwright.config";

export default defineConfig({
  ...base,
  testDir: ".",
  testMatch: /((home-first-run|engine-setup)\.bare|tutorial\.spec)\.ts$/,
  globalSetup: undefined,
});
