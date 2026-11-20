// "Bestes Modell für deinen PC" for PCs other than the one the tests run on (X8): the server takes the
// PC from the last stored PC scan (setting `system_scan`, as "PC prüfen" writes it) before it reads the
// live one. Each test stores a scan of another PC in the test database (`RP_E2E_DATA`, never the
// owner's), checks the page against the server's own answer for it and puts the old scan back.
// Profiles: no GPU with 32 GB RAM, an 8 GB NVIDIA card with 32 GB RAM, and no GPU with 4 GB RAM
// (nothing fits). `RP_E2E_SHOTS` (optional): review screenshots go there.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { Locator, Page } from "@playwright/test";
import type { CatalogModel, Pick as WordsPick } from "../src/models/types";
import { reasonText } from "../src/models/words";
import { api } from "./api";
import { dataDir } from "./db";
import { expect, isPhone, test } from "./fixtures";

type Tier = "small" | "balanced" | "large";
type Fit = "fits" | "cpu_help" | "too_big";
type Pick = { id: string; tier: Tier; reason_code: string; fit: Fit; n_cpu_moe: number; gpu_gb: number; ram_gb: number; ctx_max: number };
type Purpose = "roleplay" | "tempo" | "context";
type Catalog = {
  pc: { vram_bytes: number; source: string; backend: string | null } | null;
  vram_budget_gb: number | null;
  best: (Record<Purpose, Pick[]> & { empty: Record<Purpose, "nothing_fits" | "all_shown" | null> }) | null;
  models: { id: string; name: string; quants: Record<Tier, { fit: Fit | null; gpu_gb: number | null; ram_gb: number | null } | null> }[];
};

const GiB = 2 ** 30;
const SCANS = {
  "cpu-32": { gpus: [], ram_total: 32 * GiB, backend: { suggested: "cpu" } },
  "nvidia-8": { gpus: [{ name: "GeForce RTX 4060", vendor: "nvidia", vram_total: 8 * GiB }], ram_total: 32 * GiB, backend: { suggested: "cuda" } },
  "cpu-4": { gpus: [], ram_total: 4 * GiB, backend: { suggested: "cpu" } },
} as const;
const SHOTS = process.env.RP_E2E_SHOTS;
const COLUMNS: [Purpose, string][] = [
  ["roleplay", "Rollenspiel"],
  ["tempo", "Tempo"],
  ["context", "Kontext"],
];

/** The stored scan (null: none), and a write of one (null: remove it). */
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

const one = new Intl.NumberFormat("de-DE", { minimumFractionDigits: 1, maximumFractionDigits: 1 });
const gb = (n: number) => `${one.format(n)} GB`;
const ctx = (n: number) => `${Math.round(n / 1024)}k`;

/** What the page must say about where a pick runs (the server's numbers, in the page's words). */
function place(p: Pick): string {
  const where = p.fit === "fits" ? `ca. ${gb(p.gpu_gb)} im Grafikspeicher` : p.gpu_gb ? `ca. ${gb(p.gpu_gb)} im Grafikspeicher + ${gb(p.ram_gb)} im Arbeitsspeicher` : `ca. ${gb(p.ram_gb)} im Arbeitsspeicher`;
  return p.reason_code === "room_for_context" ? where : `${where} · Kontext bis ${ctx(p.ctx_max)}`;
}

/** The reason the page must show: the page's own words for the server's code (`src/models/words.ts`,
 *  each sentence checked in r1-fit-reasons.spec.ts). Without a GPU nothing is called fast. */
function reason(p: Pick, cat: Catalog): string {
  const m = cat.models.find((x) => x.id === p.id) as unknown as CatalogModel;
  return reasonText(p as unknown as WordsPick, m, cat.vram_budget_gb ?? 0);
}

async function column(page: Page, name: string): Promise<Locator> {
  const best = page.locator('[data-spot="best-models"]');
  if (isPhone(page)) await best.getByRole("group", { name: "Zweck" }).getByRole("button", { name, exact: true }).click();
  return best.getByRole("region", { name, exact: true });
}

async function shot(page: Page, name: string, project: string) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.locator('[data-spot="best-models"]').scrollIntoViewIfNeeded();
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(SHOTS, `x8-${name}-${project}.png`) });
}

test.describe("Bestes Modell für andere PCs", () => {
  test.skip(!dataDir, "needs RP_E2E_DATA (the test server's --data-dir)");
  let saved: string | null = null;
  test.beforeEach(() => {
    saved = scanSetting();
  });
  test.afterEach(() => {
    scanSetting(saved);
  });

  for (const [name, scan] of Object.entries(SCANS)) {
    test(`PC-Profil ${name}: Picks, Ort im Speicher, leere Spalten`, async ({ page, open }, info) => {
      scanSetting(JSON.stringify(scan));
      const cat = await api<Catalog>("GET", "/models/catalog");
      expect(cat.pc?.source).toBe("scan");
      const best = cat.best!;
      const all = [...best.roleplay, ...best.tempo, ...best.context];
      if (name === "cpu-4") expect(all).toHaveLength(0);
      else expect(best.roleplay.length).toBeGreaterThan(0);
      // Without a GPU every pick runs in the RAM; the 8 GB card puts at least one MoE model's experts there.
      if (name.startsWith("cpu")) expect(all.every((p) => p.fit === "cpu_help" && p.gpu_gb === 0)).toBe(true);
      if (name === "nvidia-8") expect(all.some((p) => p.fit === "cpu_help" && p.gpu_gb > 0 && p.n_cpu_moe > 0)).toBe(true);

      await open("/settings/suggestions");
      const panel = page.locator('[data-spot="best-models"]');
      const head = page.locator(".set-row").filter({ has: panel }).locator(".set-label p");
      await expect(head).toContainText("Letzter PC-Scan:");
      await expect(head).not.toContainText("Dateigröße plus");
      if (cat.vram_budget_gb) await expect(head).toContainText(`Vom Grafikspeicher zählen ${gb(cat.vram_budget_gb)}`);
      else await expect(head).toContainText("Ohne Grafikkarte läuft das Modell auf dem Prozessor");

      if (all.length === 0) {
        // One sentence, not three empty columns.
        await expect(panel.locator(".x8-empty")).toHaveCount(1);
        await expect(panel.locator(".x8-empty")).toHaveText("Kein Modell aus unserer Auswahl passt auf diesen PC, auch nicht mit Hilfe des Arbeitsspeichers.");
        await expect(panel.getByRole("region")).toHaveCount(0);
      }
      for (const [key, title] of all.length ? COLUMNS : []) {
        const col = await column(page, title);
        const picks = best[key];
        const items = col.getByRole("listitem");
        await expect(items).toHaveCount(picks.length);
        if (picks.length === 0) {
          const why = best.empty[key];
          await expect(col.locator(".x8-empty")).toHaveText(why === "all_shown" ? "Alle Modelle, die auf deinen PC passen, stehen schon in den anderen Spalten." : /Kein Modell/);
        }
        for (const [i, p] of picks.entries()) {
          const it = items.nth(i);
          await expect(it).toHaveAccessibleName(cat.models.find((m) => m.id === p.id)!.name);
          await expect(it.locator(".best-why")).toHaveText(reason(p, cat));
          if (!cat.vram_budget_gb) expect(reason(p, cat)).not.toMatch(/^Schnell/);
          await expect(it.locator(".x8-place")).toHaveText(place(p));
          if (p.n_cpu_moe > 0) await expect(it.locator(".x8-place")).toHaveAttribute("title", new RegExp(`ersten ${p.n_cpu_moe} Schichten`));
        }
      }
      if (isPhone(page) && all.length) await column(page, "Rollenspiel");
      // The page never scrolls sideways.
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
      await shot(page, name, info.project.name);

      // A catalog card's pill for a tier with RAM help names the RAM part ("ca. 4,6 GB + 9,8 GB RAM");
      // without a GPU it says the processor runs it.
      const pills = page.locator('[data-spot="suggestions"] .fit-pill.fit-cpu_help');
      if (name === "cpu-32") expect(await pills.count()).toBeGreaterThan(0);
      for (const t of await pills.allTextContents())
        expect(t).toMatch(cat.vram_budget_gb ? /^(Passt mit CPU-Hilfe · ca\. [\d,]+ GB \+ [\d,]+ GB RAM|Läuft auf dem Prozessor · ca\. [\d,]+ GB RAM)$/ : /^Läuft auf dem Prozessor · ca\. [\d,]+ GB RAM$/);
    });
  }
});
