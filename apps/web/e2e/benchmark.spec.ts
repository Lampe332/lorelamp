// P6 benchmark: in the tab "Modell", "Beste Einstellung finden" starts a benchmark of the active
// profile, shows its progress and then the table and three recommendations; "Setzen" writes one into
// the profile, which then shows "gemessen ✓" in the card "Dein Modell" and in the model status. Also: Ctrl+K finds it, and a cancelled
// run leaves the profile as it was.
// Needs a test server that manages the model (`--manage-engine`, see README) and, so a run takes
// seconds instead of minutes, shortened runs in the server's environment:
// RP_BENCH_CTX=2048,4096 RP_BENCH_PROMPT_TOKENS=256 RP_BENCH_GEN_TOKENS=16 RP_BENCH_BUDGET_SECS=120.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { api } from "./api";
import { expect, isPhone, modelStatus, test } from "./fixtures";

type Profile = { id: number; name: string; load_args: string[]; measured: boolean; active: boolean };
type Engine = { status: string; managed: boolean; unloaded: boolean; model: { ctx: number | null }; bench_profile_id?: number | null };
type Bench = { running: unknown; result: { rows: unknown[]; picks: { purpose: string; row: number }[] } | null };

const engine = () => api<Engine>("GET", "/engine");
const active = async () => (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles.find((p) => p.active)!;
const SHOTS = process.env.RP_E2E_SHOTS;

let original: Profile | null = null;

async function shot(page: Page, name: string, theme: string) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: join(SHOTS, `${name}-${isPhone(page) ? "375" : "1920"}-${theme}.png`), fullPage: false });
}

/** No benchmark runs and the model answers (with the context `ctx`, when given). */
async function settled(ctx?: number) {
  await expect
    .poll(
      async () => {
        const e = await engine();
        return e.status === "ready" && !e.bench_profile_id && (ctx === undefined || e.model.ctx === ctx);
      },
      { timeout: 180_000, intervals: [500, 1000, 2000] },
    )
    .toBe(true);
}

test.beforeEach(async () => {
  test.skip(!(await engine()).managed, "the test server does not manage the model (start it with --manage-engine)");
  await settled();
  original ??= await active();
});

// The profile gets its own start options back (and "geschätzt"), so later tests see the usual model.
test.afterEach(async () => {
  if (!original) return;
  const now = await active();
  if (JSON.stringify(now.load_args) !== JSON.stringify(original.load_args) || now.measured !== original.measured) {
    await api("PUT", `/models/profiles/${original.id}`, { load_args: original.load_args, measured: original.measured });
  }
  await settled(4096);
});

async function openBench(page: Page, open: (hash: string) => Promise<void>) {
  await open("#/settings/model");
  await page.getByRole("button", { name: "Beste Einstellung finden" }).click();
  const bench = page.locator('[data-spot="benchmark"]');
  await bench.scrollIntoViewIfNeeded();
  await expect(bench.getByRole("heading", { name: "Einstellungen messen" })).toBeVisible();
  return bench;
}

test("Benchmark: starten, Fortschritt, Tabelle, Empfehlungen, Setzen macht das Profil „gemessen ✓“", async ({ page, open, theme }) => {
  test.setTimeout(300_000);
  const p = await active();
  const bench = await openBench(page, open);
  await expect(bench.getByText(/Dauert bis zu 5 Minuten/)).toBeVisible();
  const had = (await api<Bench>("GET", `/models/profiles/${p.id}/benchmark`)).result;
  if (!had || theme === "graphit") {
    // A real run (the stored result of an earlier project is reused in the other themes).
    await bench.getByRole("button", { name: /Benchmark starten|Neu messen/ }).click();
    const progress = bench.getByRole("status", { name: "Benchmark läuft" });
    await expect(progress).toBeVisible();
    await expect(progress).toContainText(/von höchstens 2:00/);
    await expect(bench.getByRole("button", { name: "Abbrechen" })).toBeVisible();
    // The first row comes after a load, a warm-up and at least three runs: on a busy PC that can take
    // most of the 2:00 budget.
    await expect(bench.locator(".bm-table tbody tr").first()).toBeVisible({ timeout: 150_000 });
    await shot(page, "benchmark-running", theme);
    await expect(progress).toBeHidden({ timeout: 180_000 });
  }
  // Table: 2 steps × 2 KV types, all fit on the CPU; three recommendations.
  await expect(bench.locator(".bm-table tbody tr")).toHaveCount(4);
  await expect(bench.locator(".bm-table thead")).toContainText("Kontext");
  await expect(bench.locator(".bm-table thead")).toContainText("Überlauf");
  await expect(bench.locator(".bm-table td.ok")).toHaveCount(4);
  // One card per chosen row: recommendations that chose the same row share one card that names them all.
  const stored = (await api<Bench>("GET", `/models/profiles/${p.id}/benchmark`)).result!;
  const cards = bench.locator(".bm-pick");
  await expect(cards).toHaveCount(new Set(stored.picks.map((x) => x.row)).size);
  for (const [purpose, label] of [
    ["speed", "Tempo"],
    ["balanced", "Ausgewogen"],
    ["context", "Kontext"],
  ])
    await expect(bench.locator(`.bm-pick[data-purposes~="${purpose}"] .bm-pick-label`)).toContainText(label);
  await expect(bench.getByText(/Zuletzt gemessen/)).toBeVisible();
  await shot(page, "benchmark-result", theme);
  if (isPhone(page)) {
    await bench.locator(".bm-table").scrollIntoViewIfNeeded();
    await shot(page, "benchmark-table", theme);
  }

  // "Setzen" on Tempo: the start options change, the profile is measured, the model restarts with it.
  const result = (await api<Bench>("GET", `/models/profiles/${p.id}/benchmark`)).result!;
  const row = result.rows[result.picks.find((x) => x.purpose === "speed")!.row] as { ctx: number };
  await bench.getByRole("button", { name: /^Tempo( · .+)? setzen$/ }).click();
  await expect(bench.locator('.bm-pick[data-purpose="speed"]').getByText("Gesetzt")).toBeVisible({ timeout: 30_000 });
  await expect.poll(async () => (await active()).measured).toBe(true);
  expect((await active()).load_args.join(" ")).toContain(`-c ${row.ctx}`);
  await expect(page.locator('[data-spot="model"]').getByText("gemessen ✓")).toBeVisible();
  await settled(row.ctx);
  if (!isPhone(page)) await expect(modelStatus(page)).toContainText("gemessen ✓", { timeout: 30_000 });
  await bench.scrollIntoViewIfNeeded();
  await shot(page, "benchmark-set", theme);
});

test("Strg K: „Benchmark“ und „Einstellungen messen“ führen zum Benchmark", async ({ page, open }) => {
  await open("#/");
  for (const [words, label] of [
    ["benchmark", "Benchmark"],
    ["einstellungen messen", "Einstellungen messen"],
  ]) {
    if (isPhone(page)) await page.locator("header.top").getByRole("button", { name: "Suchen und Schnellaktionen" }).click();
    else await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
    await expect(palette).toBeVisible();
    await page.keyboard.type(words);
    await palette.getByRole("option").filter({ hasText: label }).first().click();
    await expect(page).toHaveURL(/#\/settings\/model/);
    const bench = page.locator('[data-spot="benchmark"]');
    await expect(bench).toBeInViewport();
    await expect(bench.getByRole("button", { name: /Benchmark starten|Neu messen/ })).toBeVisible();
    await page.goto(`/?token=${process.env.RP_E2E_TOKEN ?? ""}#/`);
  }
});

test("Abbrechen: der Lauf endet, das Profil bleibt, wie es war", async ({ page, open, theme }) => {
  test.skip(isPhone(page) || theme !== "graphit", "one cancelled run is enough");
  test.setTimeout(240_000);
  const before = await active();
  const bench = await openBench(page, open);
  await bench.getByRole("button", { name: /Benchmark starten|Neu messen/ }).click();
  await expect(bench.getByRole("status", { name: "Benchmark läuft" })).toBeVisible();
  await bench.getByRole("button", { name: "Abbrechen" }).click();
  await expect(bench.getByText("Abgebrochen. Das Profil ist unverändert.")).toBeVisible({ timeout: 60_000 });
  const after = await active();
  expect([after.load_args, after.measured]).toEqual([before.load_args, before.measured]);
  await settled(4096);
});
