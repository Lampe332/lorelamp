import { defineConfig } from "@playwright/test";

// UI safety net for the rework. Runs against a separately started test server (see e2e/README.md);
// URL and token come from RP_E2E_URL / RP_E2E_TOKEN. Uses the installed Edge, no browser download.
//
// Theme and start layout are server-wide settings, so everything runs in one worker, one test after
// the other; each test puts its project's theme in place first (see e2e/fixtures.ts).

const viewports = [
  { name: "phone", viewport: { width: 375, height: 812 } },
  { name: "desktop", viewport: { width: 1920, height: 1080 } },
];
const themes = ["graphit", "papier"];

export default defineConfig({
  testDir: "e2e",
  globalSetup: "./e2e/global-setup.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 90_000,
  expect: {
    timeout: 10_000,
    toHaveScreenshot: { maxDiffPixelRatio: 0.02, threshold: 0.3, animations: "disabled", caret: "hide" },
  },
  reporter: [["list"]],
  snapshotPathTemplate: "{testDir}/__screenshots__/{testFilePath}/{arg}-{projectName}{ext}",
  use: {
    baseURL: process.env.RP_E2E_URL ?? "http://127.0.0.1:7861",
    channel: "msedge",
    headless: true,
    locale: "de-DE",
    timezoneId: "Europe/Zurich",
    trace: "retain-on-failure",
  },
  projects: viewports.flatMap((v) =>
    themes.map((theme) => ({
      name: `${v.name}-${theme}`,
      use: { viewport: v.viewport, theme },
    })),
  ),
});
