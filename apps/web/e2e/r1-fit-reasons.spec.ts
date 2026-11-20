// The tab "Vorschläge" after the model review (R1): the filter "Passt auf meinen PC" keeps exactly the
// tiers the server's pill calls fitting (also with RAM help, also on a PC without a GPU), every reason
// code of "Bestes Modell" has its own honest sentence, and "Benutzen" says how a new model starts.
// The PCs are simulated as in best-pc.spec.ts: a stored PC scan (setting `system_scan`) in the test
// database, the server's real answer, the old scan put back. The texts are pure functions
// (`src/models/words.ts`) and are checked directly. `RP_E2E_SHOTS` (optional): review screenshots.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { Locator, Page } from "@playwright/test";
import type { CatalogModel, Pick, Placement, Reason, Tier } from "../src/models/types";
import { fallbackText, fitName, placementText, reasonText } from "../src/models/words";
import { api } from "./api";
import { dataDir } from "./db";
import { expect, isPhone, test } from "./fixtures";

type Catalog = { pc: { vram_bytes: number; source: string } | null; vram_budget_gb: number | null; models: CatalogModel[] };

const GiB = 2 ** 30;
const SCANS = {
  "cpu-32": { gpus: [], ram_total: 32 * GiB, backend: { suggested: "cpu" } },
  "nvidia-8": { gpus: [{ name: "GeForce RTX 4060", vendor: "nvidia", vram_total: 8 * GiB }], ram_total: 32 * GiB, backend: { suggested: "cuda" } },
} as const;
const TIERS: Tier[] = ["small", "balanced", "large"];
const SHOTS = process.env.RP_E2E_SHOTS;

function scanSetting(value?: string | null): string | null {
  const db = new DatabaseSync(join(dataDir, "rp-app.db"));
  try {
    db.exec("PRAGMA busy_timeout = 5000");
    if (value === undefined) return (db.prepare("SELECT value FROM settings WHERE key = 'system_scan'").get() as { value: string } | undefined)?.value ?? null;
    if (value === null) db.prepare("DELETE FROM settings WHERE key = 'system_scan'").run();
    else db.prepare("INSERT INTO settings (key, value) VALUES ('system_scan', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(value);
    return value;
  } finally {
    db.close();
  }
}

/** The filters: the column beside the cards, or the sheet on a phone (opened here). */
async function filters(page: Page): Promise<Locator> {
  if (!isPhone(page)) return page.locator('[data-spot="suggestions"]').getByRole("complementary", { name: "Filter" });
  const sheet = page.getByRole("dialog", { name: "Filter" });
  if (!(await sheet.isVisible())) await page.locator('[data-spot="suggestions"]').getByRole("button", { name: /^Filter( \d+)?$/ }).click();
  await expect(sheet).toBeVisible();
  return sheet;
}

// ------------------------------------------------------------------ the texts, as pure functions

const model = (over: Partial<CatalogModel>): CatalogModel => ({ name: "Testmodell", moe: false, params_total_b: 12, params_active_b: null, ...over }) as CatalogModel;
const pick = (reason_code: Reason, over: Partial<Pick> = {}): Pick => ({ id: "x", tier: "balanced", est_gb: 6, q: 4, installed: false, measured: null, reason_code, fit: "fits", n_cpu_moe: 0, gpu_gb: 6, ram_gb: 0, ctx_max: 32768, ...over });

test("Gründe: jeder Code hat einen eigenen, ehrlichen Satz", ({}, info) => {
  test.skip(info.project.name !== "desktop-graphit", "pure functions: one project is enough");
  const moe = model({ moe: true, params_total_b: 26, params_active_b: 4 });
  const dense = model({ params_total_b: 12 });
  const cases: [string, string, string][] = [
    ["best_quality", reasonText(pick("best_quality"), dense, 20), "Beste Qualität, die ganz in deinen Grafikspeicher passt"],
    ["best_quality RAM", reasonText(pick("best_quality", { fit: "cpu_help", gpu_gb: 6, ram_gb: 12 }), moe, 6.5), "Beste Qualität für deinen PC, ein Teil läuft im Arbeitsspeicher"],
    ["best_quality CPU", reasonText(pick("best_quality", { fit: "cpu_help", gpu_gb: 0, ram_gb: 9 }), dense, 0), "Beste Qualität, die in deinen Arbeitsspeicher passt"],
    ["installed_quality", reasonText(pick("installed_quality", { installed: true }), dense, 20), "Schon installiert und gut für Rollenspiel"],
    ["fast_moe", reasonText(pick("fast_moe"), moe, 20), "Schnell: läuft ganz auf der Grafikkarte und rechnet je Wort nur 4 Mrd. Parameter"],
    ["small_dense", reasonText(pick("small_dense"), dense, 20), "Schnell: klein genug für die Grafikkarte, 12 Mrd. Parameter"],
    ["measured_fast", reasonText(pick("measured_fast", { measured: { tg: 41.6, ctx: 32768 } }), dense, 20), "Gemessen: 42 Tokens/s auf deinem PC"],
    ["measured_slow", reasonText(pick("measured_slow", { measured: { tg: 6.2, ctx: 8192 } }), dense, 20), "Gemessen: nur 6 Tokens/s auf deinem PC"],
    ["partly_in_ram", reasonText(pick("partly_in_ram", { fit: "cpu_help", gpu_gb: 4.4, ram_gb: 12.7, n_cpu_moe: 27 }), moe, 4.5), "Rechnet je Wort nur 4 Mrd. Parameter, ein Teil liegt aber im Arbeitsspeicher und bremst"],
    ["cpu_moe", reasonText(pick("cpu_moe", { fit: "cpu_help", gpu_gb: 0, ram_gb: 17 }), moe, 0), "Läuft auf dem Prozessor und rechnet je Wort nur 4 Mrd. Parameter"],
    ["cpu_dense", reasonText(pick("cpu_dense", { fit: "cpu_help", gpu_gb: 0, ram_gb: 9 }), dense, 0), "Läuft auf dem Prozessor, 12 Mrd. Parameter je Wort: eher langsam"],
    ["room_for_context", reasonText(pick("room_for_context", { ctx_max: 131072 }), dense, 20), "Lässt viel Platz für lange Chats: bis 128k Kontext"],
    ["unknown", reasonText(pick("something_new" as Reason), dense, 20), "Passt auf deinen PC"],
  ];
  for (const [what, got, want] of cases) expect(got, what).toBe(want);
  // Every code says something of its own.
  const texts = cases.map(([, t]) => t);
  expect(new Set(texts).size).toBe(texts.length);
  // "Schnell" only for all on the GPU; never for RAM, the processor or a slow measurement.
  for (const code of ["partly_in_ram", "cpu_moe", "cpu_dense", "measured_slow"] as Reason[]) {
    expect(reasonText(pick(code, { measured: code === "measured_slow" ? { tg: 3, ctx: 8192 } : null }), moe, 0)).not.toMatch(/schnell/i);
  }
});

test("Benutzen: die Zeile sagt, wie das Modell startet", ({}, info) => {
  test.skip(info.project.name !== "desktop-graphit", "pure functions: one project is enough");
  const base: Placement = { fit: "cpu_help", n_cpu_moe: 25, n_gpu_layers: null, ctx: 8192, gpu_gb: 6, ram_gb: 12.1, source: "catalog", fallback: null };
  expect(placementText(base)).toBe("Startet mit ca. 6,0 GB im Grafikspeicher + 12,1 GB im Arbeitsspeicher, Kontext 8k.");
  expect(placementText({ ...base, fit: "fits", n_cpu_moe: 0, gpu_gb: 5.2, ram_gb: 0 })).toBe("Startet ganz im Grafikspeicher: ca. 5,2 GB, Kontext 8k.");
  expect(placementText({ ...base, n_cpu_moe: 0, gpu_gb: 0, ram_gb: 7.9 })).toBe("Startet auf dem Prozessor: ca. 7,9 GB im Arbeitsspeicher, Kontext 8k.");
  expect(placementText({ ...base, n_cpu_moe: 0, n_gpu_layers: 18, gpu_gb: 6.3, ram_gb: 8.4, fallback: "n_cpu_moe_unsupported" })).toBe(
    "Startet mit ca. 6,3 GB im Grafikspeicher + 8,4 GB im Arbeitsspeicher, Kontext 8k. 18 Schichten laufen auf der Grafikkarte, die übrigen auf dem Prozessor.",
  );
  expect(placementText({ ...base, fit: "too_big", n_cpu_moe: 0 })).toBe("Passt laut Schätzung nicht in den Speicher. Es startet mit den Startoptionen des bisherigen Profils, Kontext 8k.");
  expect(fallbackText("n_cpu_moe_unsupported")).toBe("Diese Engine kann die Experten nicht einzeln in den Arbeitsspeicher legen. Stattdessen liegen ganze Schichten dort, das ist langsamer.");
  expect(fallbackText(null)).toBeNull();
  expect(fallbackText("other")).toBeNull();
  // The pill on a PC without a GPU names the processor, not "CPU help".
  expect(fitName({ fit: "cpu_help", gpu_gb: 0 } as Pick)).toBe("Läuft auf dem Prozessor");
  expect(fitName({ fit: "cpu_help", gpu_gb: 6 } as Pick)).toBe("Passt mit CPU-Hilfe");
});

// ------------------------------------------------------------------ the filter on simulated PCs

test.describe("Filter „Passt auf meinen PC“ für andere PCs", () => {
  test.skip(!dataDir, "needs RP_E2E_DATA (the test server's --data-dir)");
  let saved: string | null = null;
  test.beforeEach(() => {
    saved = scanSetting();
  });
  test.afterEach(() => {
    scanSetting(saved);
  });

  for (const [name, scan] of Object.entries(SCANS)) {
    test(`PC-Profil ${name}: der Filter zeigt, was der Server passend nennt`, async ({ page, open }, info) => {
      scanSetting(JSON.stringify(scan));
      const cat = await api<Catalog>("GET", "/models/catalog");
      expect(cat.pc?.source).toBe("scan");
      const fitting = (m: CatalogModel) => TIERS.some((t) => {
        const q = m.quants[t];
        return !!q && !q.missing && (q.fit === "fits" || q.fit === "cpu_help");
      });
      const want = cat.models.filter(fitting);
      const all = cat.models.length;
      expect(want.length).toBeGreaterThan(0);
      if (name === "nvidia-8") {
        // A MoE model with its experts partly in RAM counts as fitting.
        expect(want.some((m) => m.moe && TIERS.some((t) => m.quants[t]?.fit === "cpu_help" && (m.quants[t]?.gpu_gb ?? 0) > 0))).toBe(true);
      }

      await open("/settings/suggestions");
      const panel = page.locator('[data-spot="suggestions"]');
      const cards = panel.getByRole("list", { name: "Vorschläge" }).locator(":scope > li");
      await expect(panel.locator(".sug-count")).toHaveText(`${all} von ${all}`);
      const f = await filters(page);
      const sw = f.getByRole("switch", { name: /Passt auf meinen PC/ });
      await expect(sw).toBeVisible();
      await expect(f.locator(".sug-fit")).toContainText(name.startsWith("cpu") ? "Modell und 8k Kontext im Arbeitsspeicher" : "ganz in den Grafikspeicher passen");
      // With a known PC there is no guessed GB number to set.
      await expect(f.getByRole("slider", { name: "Grafikspeicher höchstens" })).toHaveCount(0);
      const levels = f.getByRole("group", { name: "Wie es passen soll" });
      await sw.check();
      if (name.startsWith("cpu")) {
        // Without a usable graphics card there is one level only.
        await expect(levels).toHaveCount(0);
      } else {
        // Y1: two levels. First only what runs all on the card (on 8 GB that narrows the catalog),
        // then also what puts a part into the RAM.
        const onCard = cat.models.filter((m) => TIERS.some((t) => !!m.quants[t] && !m.quants[t]!.missing && m.quants[t]!.fit === "fits"));
        expect(onCard.length).toBeGreaterThan(0);
        expect(onCard.length).toBeLessThan(want.length);
        await expect(levels.getByRole("button", { name: "Ganz auf der Grafikkarte" })).toHaveAttribute("aria-pressed", "true");
        await expect(panel.locator(".sug-count")).toHaveText(`${onCard.length} von ${all}`);
        await levels.getByRole("button", { name: "Auch mit Arbeitsspeicher, langsamer" }).click();
        await expect(f.locator(".sug-fit")).toContainText("einen Teil in den Arbeitsspeicher legen");
      }
      await expect(panel.locator(".sug-count")).toHaveText(`${want.length} von ${all}`);
      if (isPhone(page)) await f.getByRole("button", { name: /Treffer zeigen$/ }).click();
      await expect(cards).toHaveCount(want.length);
      for (const m of want) await expect(cards.filter({ has: page.locator(".sg-name", { hasText: new RegExp(`^${m.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`) }) })).toHaveCount(1);
      // Each card opens on a fitting tier: its pill never says "Zu groß".
      await expect(panel.locator(".sg-cards .fit-pill.fit-too_big")).toHaveCount(0);
      if (name === "nvidia-8") {
        const moe = want.find((m) => m.moe && TIERS.some((t) => m.quants[t]?.fit === "cpu_help" && (m.quants[t]?.gpu_gb ?? 0) > 0) && !TIERS.some((t) => m.quants[t]?.fit === "fits"))!;
        if (moe) {
          const card = cards.filter({ has: page.locator(".sg-name", { hasText: moe.name }) }).first();
          await expect(card.locator(".fit-pill")).toHaveText(/^Passt mit CPU-Hilfe · ca\. [\d,]+ GB \+ [\d,]+ GB RAM$/);
          await expect(card.locator(".fit-pill")).toHaveAttribute("title", /Schichten liegen die Experten im Arbeitsspeicher/);
        }
      } else {
        for (const t of await panel.locator(".sg-cards .fit-pill").allTextContents()) expect(t).toMatch(/^Läuft auf dem Prozessor · ca\. [\d,]+ GB RAM$/);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
      if (SHOTS) {
        mkdirSync(SHOTS, { recursive: true });
        if (isPhone(page)) {
          const g = await filters(page);
          await expect(g.locator(".sug-fit")).toBeVisible();
        } else await panel.evaluate((el) => el.scrollIntoView({ block: "start" }));
        await page.evaluate(() => document.fonts.ready);
        await page.screenshot({ path: join(SHOTS, `r1-filter-${name}-${info.project.name}.png`) });
      }
      // Off again: the whole catalog.
      const g = await filters(page);
      await g.getByRole("switch", { name: /Passt auf meinen PC/ }).uncheck();
      await expect(panel.locator(".sug-count")).toHaveText(`${all} von ${all}`);
    });
  }
});
