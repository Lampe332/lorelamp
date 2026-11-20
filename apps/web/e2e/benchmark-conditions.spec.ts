// X5 honest benchmark results: after a real CPU run every measured row shows how far its runs spread
// (and a quiet "unsicher" when they spread too far), the result says what memory was free when it
// started; a result stored before these fields existed still shows cleanly; and with two results kept
// under different conditions one can switch between them, see the stops with their numbers, and
// "Setzen" takes the setting from the result shown.
// Needs what benchmark.spec.ts needs (`--manage-engine`, shortened runs: RP_BENCH_CTX=2048,4096
// RP_BENCH_PROMPT_TOKENS=256 RP_BENCH_GEN_TOKENS=16 RP_BENCH_BUDGET_SECS=120) and RP_E2E_DATA: the
// older-form and second results are written straight into the test server's settings (in the form
// the server stores them) and the stored ones are put back afterwards.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { Page } from "@playwright/test";
import { api } from "./api";
import { dataDir } from "./db";
import { expect, isPhone, test } from "./fixtures";

type Profile = { id: number; load_args: string[]; measured: boolean; active: boolean };
type Engine = { status: string; managed: boolean; model: { ctx: number | null }; bench_profile_id?: number | null };
type Limit = { what: "ram" | "commit" | "shared"; available_mib: number; needed_mib: number };
type Row = {
  ctx: number;
  kv: string;
  n_cpu_moe: number;
  pp: number | null;
  tg: number | null;
  overflow_mib: number | null;
  outcome: string;
  load_s: number | null;
  pp_runs?: number[];
  tg_runs?: number[];
  spread?: number | null;
  uncertain?: boolean;
  limit?: Limit | null;
};
type Conditions = { ram_available_mib: number | null; vram_free_mib: number | null; commit_free_mib: number | null };
type Result = { profile_id: number; at: number; duration_s: number; rows: Row[]; picks: { purpose: string; row: number }[]; moe: boolean; gpu: string | null; conditions?: Conditions | null };
type Bench = { running: unknown; result: Result | null; results: Result[] };

const engine = () => api<Engine>("GET", "/engine");
const active = async () => (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles.find((p) => p.active)!;
const bench = (id: number) => api<Bench>("GET", `/models/profiles/${id}/benchmark`);
const SHOTS = process.env.RP_E2E_SHOTS;
const GB = (mib: number) => `${new Intl.NumberFormat("de-DE", { minimumFractionDigits: mib < 10 * 1024 ? 1 : 0, maximumFractionDigits: mib < 10 * 1024 ? 1 : 0 }).format(mib / 1024)} GB`;

let original: Profile | null = null;

async function shot(page: Page, name: string, theme: string) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: join(SHOTS, `x5-${name}-${isPhone(page) ? "375" : "1920"}-${theme}.png`), fullPage: false });
}

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

/** The profile's stored benchmark settings (`bench_result_<id>`, `bench_results_<id>`): read, and written back. */
function settings(id: number) {
  const keys = [`bench_result_${id}`, `bench_results_${id}`];
  const db = () => {
    const d = new DatabaseSync(join(dataDir, "rp-app.db"));
    d.exec("PRAGMA busy_timeout = 5000");
    return d;
  };
  const read = () => {
    const d = db();
    try {
      return keys.map((k) => (d.prepare("SELECT value FROM settings WHERE key = ?").get(k) as { value: string } | undefined)?.value ?? null);
    } finally {
      d.close();
    }
  };
  const write = (values: (string | null)[]) => {
    const d = db();
    try {
      keys.forEach((k, i) => {
        const v = values[i];
        if (v === null) d.prepare("DELETE FROM settings WHERE key = ?").run(k);
        else d.prepare("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(k, v);
      });
    } finally {
      d.close();
    }
  };
  return { read, write };
}

test.beforeEach(async () => {
  test.skip(!(await engine()).managed, "the test server does not manage the model (start it with --manage-engine)");
  await settled();
  original ??= await active();
});

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
  const b = page.locator('[data-spot="benchmark"]');
  await b.scrollIntoViewIfNeeded();
  await expect(b.getByRole("heading", { name: "Einstellungen messen" })).toBeVisible();
  return b;
}

test("Echter Lauf: jede gemessene Zeile zeigt ihre Streuung, das Ergebnis die Umstände beim Start", async ({ page, open, theme }) => {
  test.setTimeout(300_000);
  const p = await active();
  const b = await openBench(page, open);
  const had = (await bench(p.id)).result;
  const run = !had?.conditions || (theme === "graphit" && !isPhone(page));
  if (run) {
    await b.getByRole("button", { name: /Benchmark starten|Neu messen/ }).click();
    const progress = b.getByRole("status", { name: "Benchmark läuft" });
    await expect(progress).toBeVisible();
    await expect(progress).toBeHidden({ timeout: 180_000 });
  }
  const res = (await bench(p.id)).result!;
  // The run stored a new result (it does not when no candidate fit, e.g. too little RAM on a busy PC).
  if (run) expect(res.at, "the benchmark run stored a new result").toBeGreaterThan(had?.at ?? 0);
  expect(res.conditions?.ram_available_mib).toBeGreaterThan(0);
  const measured = res.rows.filter((r) => r.tg !== null);
  expect(measured.length).toBeGreaterThan(0);
  const rows = b.locator(".bm-table tbody tr");
  await expect(rows).toHaveCount(res.rows.length);
  await expect(b.locator(".bm-table thead")).toContainText("Streuung");
  for (const [i, r] of res.rows.entries()) {
    const cell = rows.nth(i).locator(".x5-spread");
    if (r.tg === null) continue;
    // Every measured row: at least three runs, a spread in percent, "unsicher" exactly when the server says so.
    expect(r.tg_runs!.length).toBeGreaterThanOrEqual(3);
    await expect(cell).toContainText(`${new Intl.NumberFormat("de-DE").format(Math.round(r.spread! * 100))} %`);
    if (r.uncertain) await expect(cell.locator(".x5-unsure")).toHaveText("unsicher");
    else await expect(cell.locator(".x5-unsure")).toHaveCount(0);
  }
  const cond = b.locator(".x5-cond");
  await expect(cond).toContainText(`${GB(res.conditions!.ram_available_mib!)} RAM verfügbar`);
  // A CPU run names the RAM only.
  if (res.gpu === null) await expect(cond).not.toContainText("Grafikspeicher");
  else if (res.conditions!.vram_free_mib !== null) await expect(cond).toContainText(`${GB(res.conditions!.vram_free_mib)} Grafikspeicher frei`);
  await expect(cond).toContainText("Offene Programme belegen Speicher und drücken das Ergebnis.");
  await b.locator(".x5-about").scrollIntoViewIfNeeded();
  await shot(page, "result", theme);
  await b.locator(".bm-table").scrollIntoViewIfNeeded();
  await shot(page, "table", theme);
});

test("Alte Ergebnisform ohne Läufe, Streuung und Umstände wird weiter sauber angezeigt", async ({ page, open, theme }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA (the stored result is rewritten in the test database)");
  const p = await active();
  const now = (await bench(p.id)).result;
  test.skip(!now, "needs a stored result (the run above makes one)");
  const store = settings(p.id);
  const saved = store.read();
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  try {
    // As results were stored before X5: no runs, spread, uncertain, limit or conditions, and a stop for RAM without numbers.
    const rows: Row[] = now!.rows.map(({ ctx, kv, n_cpu_moe, pp, tg, overflow_mib, outcome, load_s }) => ({ ctx, kv, n_cpu_moe, pp, tg, overflow_mib, outcome, load_s }));
    rows.push({ ctx: 8192, kv: "f16", n_cpu_moe: 0, pp: null, tg: null, overflow_mib: null, outcome: "ram", load_s: null });
    const { profile_id, at, duration_s, picks, moe, gpu } = now!;
    store.write([JSON.stringify({ profile_id, at, duration_s, rows, picks, moe, gpu }), null]);
    const b = await openBench(page, open);
    await expect(b.locator(".bm-table tbody tr")).toHaveCount(rows.length);
    await expect(b.locator(".x5-spread")).toHaveText(Array(rows.length).fill("–"));
    await expect(b.getByText("Abbruch: wenig RAM")).toBeVisible();
    await expect(b.locator(".x5-why, .x5-cond, .x5-results, .x5-unsure")).toHaveCount(0);
    await expect(b.getByText(/Zuletzt gemessen/)).toBeVisible();
    await expect(b.locator(".bm-pick")).toHaveCount(new Set(picks.map((x) => x.row)).size); // one card per chosen row
    await shot(page, "old-form", theme);
    expect(errors).toEqual([]);
  } finally {
    store.write(saved);
  }
});

test("Zwei Ergebnisse unter verschiedenen Umständen: umschalten, Gründe mit Zahlen, Setzen aus dem gezeigten", async ({ page, open, theme }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA (the second result is written into the test database)");
  test.setTimeout(240_000);
  const p = await active();
  const { result: newest, results } = await bench(p.id);
  test.skip(!newest?.conditions, "needs a stored result with conditions (the first test makes one)");
  const store = settings(p.id);
  const saved = store.read();
  try {
    // A second result as the server stores it, measured an hour earlier with far less RAM free (apps open):
    // two measured candidates, plus three stopped or left out for memory, each with its limit. Its
    // recommendation is a candidate other than the newest result's Tempo.
    const speedPick = newest!.picks.find((x) => x.purpose === "speed");
    const speedRow = speedPick ? newest!.rows[speedPick.row] : undefined;
    const ok = (ctx: number, kv: string, tg_runs: number[], spread: number, uncertain: boolean): Row => ({
      ctx,
      kv,
      n_cpu_moe: 0,
      pp: 702.4,
      tg: [...tg_runs].sort((a, b) => a - b)[Math.floor(tg_runs.length / 2)],
      overflow_mib: null,
      outcome: "ok",
      load_s: 2.1,
      pp_runs: tg_runs.map(() => 702.4),
      tg_runs,
      spread,
      uncertain,
      limit: null,
    });
    const stop = (ctx: number, kv: string, outcome: string, limit: Limit): Row => ({ ctx, kv, n_cpu_moe: 0, pp: null, tg: null, overflow_mib: null, outcome, load_s: null, pp_runs: [], tg_runs: [], spread: null, uncertain: false, limit });
    const rows: Row[] = [
      ok(2048, "f16", [19.6, 20.4, 21.1], 0.074, false),
      ok(4096, "q8_0", [17.2, 21.9, 18.4, 24.0, 19.3, 18.8], 0.36, true),
      stop(8192, "q8_0", "ram", { what: "ram", available_mib: 2150, needed_mib: 3482 }),
      stop(8192, "f16", "ram", { what: "commit", available_mib: 1843, needed_mib: 2600 }),
      stop(16384, "f16", "skipped", { what: "ram", available_mib: 1900, needed_mib: 4200 }),
      stop(16384, "q8_0", "ram", { what: "ram", available_mib: 1433, needed_mib: 1450 }),
      stop(32768, "q8_0", "shared", { what: "shared", available_mib: 4096, needed_mib: 5325 }),
    ];
    const other = rows.findIndex((r) => r.outcome === "ok" && (r.ctx !== speedRow?.ctx || r.kv !== speedRow?.kv));
    const c = newest!.conditions!;
    const older: Result = {
      ...newest!,
      at: newest!.at - 3600,
      rows,
      picks: ["speed", "balanced", "context"].map((purpose) => ({ purpose, row: other })),
      conditions: { ram_available_mib: Math.max(3000, c.ram_available_mib! - 9000), vram_free_mib: c.vram_free_mib, commit_free_mib: c.commit_free_mib },
    };
    store.write([saved[0], JSON.stringify([newest, older])]);
    expect((await bench(p.id)).results).toHaveLength(2);

    const b = await openBench(page, open);
    const group = b.getByRole("group", { name: "Ergebnisse unter verschiedenen Umständen" });
    const options = group.getByRole("button");
    await expect(options).toHaveCount(2);
    await expect(options.nth(0)).toHaveAttribute("aria-pressed", "true");
    await expect(options.nth(0)).toContainText(`${GB(c.ram_available_mib!)} RAM`);
    await expect(options.nth(1)).toContainText(`${GB(older.conditions!.ram_available_mib!)} RAM`);
    await expect(options.nth(1)).toContainText(`Tempo ${new Intl.NumberFormat("de-DE").format(Math.round(rows[other].tg! * 10) / 10)} Tokens/s`);
    await expect(b.getByText(/Zuletzt gemessen/)).toBeVisible();
    await expect(b.locator(".bm-table tbody tr")).toHaveCount(newest!.rows.length);
    await group.scrollIntoViewIfNeeded();
    await shot(page, "switch-newest", theme);

    // The older one: its conditions, its rows with the stops and their numbers.
    await options.nth(1).click();
    await expect(options.nth(1)).toHaveAttribute("aria-pressed", "true");
    await expect(options.nth(0)).toHaveAttribute("aria-pressed", "false");
    await expect(b.getByText(/^Gemessen /)).toBeVisible();
    await expect(b.locator(".x5-cond")).toContainText(`${GB(older.conditions!.ram_available_mib!)} RAM verfügbar`);
    await expect(b.locator(".bm-table tbody tr")).toHaveCount(rows.length);
    await expect(b.getByText("RAM: 2,1 GB verfügbar, 3,4 GB nötig")).toBeVisible();
    await expect(b.getByText("Commit: 1,8 GB frei, 2,5 GB nötig")).toBeVisible();
    await expect(b.getByText("Abbruch: Commit-Limit")).toBeVisible();
    await expect(b.getByText("RAM: 1,9 GB verfügbar, 4,1 GB nötig")).toBeVisible();
    // Both round to 1,4 GB: in MB, so the numbers never read as equal.
    await expect(b.getByText("RAM: 1.433 MB verfügbar, 1.450 MB nötig")).toBeVisible();
    await expect(b.getByText("Abbruch: geteilter Speicher")).toBeVisible();
    await expect(b.getByText("Geteilter GPU-Speicher: 5,2 GB belegt, Grenze 4,0 GB")).toBeVisible();
    await expect(b.getByText(/Commit = was Windows aus RAM und Auslagerungsdatei/)).toBeVisible();
    await group.scrollIntoViewIfNeeded();
    await shot(page, "switch-older", theme);
    await b.locator(".bm-table").scrollIntoViewIfNeeded();
    // Every stop reason reads in full inside the visible part of the table (on a phone it wraps), no sideways scrolling.
    const wrap = (await b.locator(".bm-table-wrap").boundingBox())!;
    for (const why of await b.locator(".x5-why").all()) {
      const box = (await why.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(wrap.x - 0.5);
      expect(box.x + box.width).toBeLessThanOrEqual(wrap.x + wrap.width + 0.5);
    }
    await shot(page, "limits", theme);

    // "Setzen" takes the shown (older) result's Tempo, not the newest one's.
    if (!isPhone(page) && theme === "graphit") {
      await b.getByRole("button", { name: /^Tempo( · .+)? setzen$/ }).click();
      await expect(b.locator('.bm-pick[data-purpose="speed"]').getByText("Gesetzt")).toBeVisible({ timeout: 30_000 });
      const after = await active();
      expect(after.measured).toBe(true);
      const args = after.load_args.join(" ");
      expect(args).toContain(`-c ${rows[other].ctx}`);
      expect(args).toContain(`-ctk ${rows[other].kv} -ctv ${rows[other].kv}`);
      await settled(rows[other].ctx);
      // Back on the newest: its own Tempo is not what the profile has now.
      await options.nth(0).click();
      await expect(b.locator('.bm-pick[data-purpose="speed"]').getByRole("button", { name: /^Tempo( · .+)? setzen$/ })).toBeVisible();
    }
  } finally {
    store.write(saved);
  }
  // Untouched results: the list as before (one entry, or what an earlier run kept).
  expect((await bench(p.id)).results).toHaveLength(results.length);
});
