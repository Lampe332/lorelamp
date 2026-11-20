// "Engine einrichten" in the first steps of an empty installation: one press picks the build for this
// PC from the newest release, downloads, checks and self-tests it, then leads on to the model
// suggestions. Runs only through e2e/first-run.config.ts against the empty server (see there), started
// additionally with RP_GITHUB_API=http://127.0.0.1:<RP_E2E_GH_PORT> (the stand-in in
// github-standin.ts, never GitHub) and run with RP_E2E_DATA (its database: the test stores a PC scan
// without a graphics card, so the CPU build is the one that fits) and RP_E2E_LLAMA_CPU (a folder with
// an official CPU build of llama.cpp; the stand-in packs it). Engine and scan are removed afterwards.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { api } from "./api";
import { dataDir } from "./db";
import { expect, test } from "./fixtures";
import { CPU_DIR, startGithubStandin, TAG } from "./github-standin";

const SHOTS = process.env.RP_E2E_SHOTS;

/** A PC without a graphics card: the scan suggests the CPU build. */
const CPU_PC = {
  gpus: [],
  ram_total: 16 * 1024 ** 3,
  ram_free: 10 * 1024 ** 3,
  commit: { limit: 32 * 1024 ** 3, used: 8 * 1024 ** 3, warning: null },
  cpu: { name: "Test-CPU", cores: 8, threads: 16 },
  backend: { suggested: "cpu", cuda: null, gpu_index: null, reason_code: "no_gpu", reason: "Keine Grafikkarte", rocm_runtime_found: false, rocm_runtime_path: null },
  scanned_at: 1_763_000_000,
};

function setScan(value: string | null) {
  const db = new DatabaseSync(join(dataDir, "rp-app.db"));
  try {
    db.exec("PRAGMA busy_timeout = 5000");
    if (value === null) db.prepare("DELETE FROM settings WHERE key = 'system_scan'").run();
    else db.prepare("INSERT INTO settings (key, value) VALUES ('system_scan', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(value);
  } finally {
    db.close();
  }
}

type Engine = { id: number; exe: string; origin: string };
const engines = async () => (await api<{ engines: Engine[] }>("GET", "/models/engines")).engines;

let stop: (() => Promise<void>) | null = null;
/** The welcome tour's step (Q9): an empty server opens it over the start screen, so it waits meanwhile. */
let tour = "";

test.beforeAll(async () => {
  test.skip(!dataDir || !CPU_DIR, "needs RP_E2E_DATA and RP_E2E_LLAMA_CPU (see the head of this file)");
  if ((await engines()).length > 0) throw new Error("engine-setup test needs a server without engines (the empty first-run server)");
  tour = (await api<{ step: string }>("GET", "/tutorial")).step;
  await api("PUT", "/tutorial", { step: "done" });
  stop = await startGithubStandin();
  setScan(JSON.stringify(CPU_PC));
});

test.afterAll(async () => {
  for (const e of await engines()) await api("DELETE", `/models/engines/${e.id}`);
  if (dataDir) setScan(null);
  if (tour) await api("PUT", "/tutorial", { step: tour });
  await stop?.();
});

test("Erster Start: Modell einrichten holt erst das Programm mit einem Klick, dann weiter zum Modell", async ({ page, open }) => {
  test.skip(test.info().project.name !== "desktop-graphit" && test.info().project.name !== "phone-papier", "one download per theme pair is enough");
  await open("/");
  const first = page.getByRole("region", { name: "Hier beginnt deine erste Geschichte" });
  const setup = first.getByRole("group", { name: "Modell einrichten" });
  await expect(setup.getByRole("button", { name: "Modell einrichten" })).toBeVisible();
  // One sentence and one button, in plain words: no second way that would need the program first.
  await expect(setup).toContainText("Dafür lädt Lorelamp zuerst das Programm, das Modelle auf deinem PC ausführt.");
  await expect(setup).not.toContainText("Engine");
  await expect(first.getByRole("link", { name: "Modell einrichten" })).toHaveCount(0);
  if (SHOTS) {
    mkdirSync(SHOTS, { recursive: true });
    await page.screenshot({ path: `${SHOTS}/engine-setup-${test.info().project.name}.png`, animations: "disabled" });
  }
  await setup.getByRole("button", { name: "Modell einrichten" }).click();
  // Download, unpack, self-test: the CPU build of the stand-in's release.
  await expect(setup.getByRole("status")).toContainText(`Installiert: llama.cpp ${TAG} (CPU)`, { timeout: 60_000 });
  await expect(setup.getByRole("status")).toContainText("Selbsttest bestanden");
  const installed = await engines();
  expect(installed).toHaveLength(1);
  expect(installed[0].exe).toContain(`${TAG}-cpu`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/engine-setup-done-${test.info().project.name}.png`, animations: "disabled" });
  await setup.getByRole("link", { name: "Weiter: Modell aussuchen" }).click();
  await expect(page).toHaveURL(/#\/settings\/suggestions$/);
  await api("DELETE", `/models/engines/${installed[0].id}`);
});
