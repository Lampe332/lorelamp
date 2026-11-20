// The settings tab "Vorschläge": "Bestes Modell für deinen PC" shows three columns with reasons and
// "Nur unzensierte" changes them, every filter narrows the catalog and "zurücksetzen" lifts it, the
// sort orders it, an installed file offers "Benutzen", a card starts a download that the sidebar
// (phone: top bar) counts, pause and resume work, a pasted Hugging Face link offers three sizes, and
// Ctrl+K finds the tab, the filters and the switch. Downloads go to a local stand-in for Hugging Face
// (e2e/hf-standin.ts, tiny files); without it (`RP_HF_BASE` not set on the test server) the download
// tests skip themselves.

import { rmSync } from "node:fs";
import { join } from "node:path";
import type { Server } from "node:http";
import type { Locator, Page } from "@playwright/test";
import type { CatalogModel, Pick as WordsPick, Placement } from "../src/models/types";
import { placementText, reasonText } from "../src/models/words";
import { api } from "./api";
import { expect, isPhone, test } from "./fixtures";
import { writeMiniGguf } from "./gguf";
import { CATALOG_REPO, HF_PORT, LINK_REPO, startHfStandIn } from "./hf-standin";

type Tier = "small" | "balanced" | "large";
type Quant = { file: string; est_gb: number; missing: boolean; installed: boolean; fit: "fits" | "cpu_help" | "too_big" | null };
type Model = {
  id: string;
  name: string;
  repo: string;
  base: string;
  moe: boolean;
  params_total_b: number | null;
  context_max: number | null;
  rp: number;
  de: number;
  q: number;
  uncensored: number;
  license: string | null;
  live: boolean;
  quants: Record<Tier, Quant | null>;
};
type Pick = { id: string; tier: Tier; reason_code: string };
type Best = { roleplay: Pick[]; tempo: Pick[]; context: Pick[] };
type Catalog = { source: string; syncing: boolean; vram_budget_gb: number | null; best: Best | null; models: Model[] };
type Download = { id: string; repo: string; state: string };
type Profile = { id: number; model_file: string; active: boolean };

const catalog = (uncensored = false) => api<Catalog>("GET", uncensored ? "/models/catalog?uncensored=1" : "/models/catalog");
const downloads = async () => (await api<{ downloads: Download[]; folder: string }>("GET", "/models/downloads")).downloads;
const SHOTS = process.env.RP_E2E_SHOTS; // folder for review screenshots (optional)

let hf: Server | null = null;
let viaStandIn = false;
let cat: Catalog;

test.beforeAll(async () => {
  hf = await startHfStandIn().catch(() => null);
  let c = await catalog();
  viaStandIn = hf !== null && c.source.startsWith(`http://127.0.0.1:${HF_PORT}`);
  await expect.poll(async () => (c = await catalog()).syncing, { timeout: 30_000 }).toBe(false);
  cat = c;
  viaStandIn &&= c.models.some((m) => m.repo === CATALOG_REPO && m.live);
});

test.afterAll(async () => {
  await new Promise<void>((r) => (hf ? hf.close(() => r()) : r()));
});

/** No download left over and no model file from an earlier test. */
async function clean() {
  for (const d of await downloads()) await api("DELETE", `/models/downloads/${d.id}`);
  const data = process.env.RP_E2E_DATA;
  if (data) for (const repo of [CATALOG_REPO, LINK_REPO]) rmSync(join(data, "models", ...repo.split("/")), { recursive: true, force: true });
}

const shot = async (page: Page, name: string, project: string) => {
  if (SHOTS) await page.screenshot({ path: join(SHOTS, `${name}-${project}.png`), fullPage: false, animations: "disabled" });
};

const TIER_NAME: Record<Tier, string> = { small: "Klein", balanced: "Ausgewogen", large: "Groß" };
/** The page's sentence for a pick's reason code (`src/models/words.ts`; every code in r1-fit-reasons.spec.ts). */
const reasonOf = (p: Pick) => reasonText(p as unknown as WordsPick, cat.models.find((m) => m.id === p.id) as unknown as CatalogModel, cat.vram_budget_gb ?? 0);
const COLUMNS: [keyof Best, string][] = [
  ["roleplay", "Rollenspiel"],
  ["tempo", "Tempo"],
  ["context", "Kontext"],
];
const nameOf = (id: string) => cat.models.find((m) => m.id === id)!.name;

/** The column of one purpose; on a phone its tab is chosen first. */
async function column(page: Page, name: string): Promise<Locator> {
  const best = page.locator('[data-spot="best-models"]');
  if (isPhone(page)) await best.getByRole("group", { name: "Zweck" }).getByRole("button", { name, exact: true }).click();
  return best.getByRole("region", { name, exact: true });
}

test("Bestes Modell: drei Spalten mit Gründen, „Nur unzensierte“ ändert die Wahl", async ({ page, open }, info) => {
  test.skip(!cat.best, "the test server could not read the PC");
  await open("/settings/suggestions");
  for (const [key, name] of COLUMNS) {
    const col = await column(page, name);
    await expect(col).toBeVisible();
    const picks = cat.best![key];
    const items = col.getByRole("listitem");
    await expect(items).toHaveCount(picks.length);
    for (const [i, p] of picks.entries()) {
      const it = items.nth(i);
      await expect(it).toHaveAccessibleName(nameOf(p.id));
      await expect(it).toContainText(TIER_NAME[p.tier]);
      await expect(it.locator(".best-why")).toHaveText(reasonOf(p));
      await expect(it.getByRole("img", { name: /^Rollenspiel \d von 5$/ })).toBeVisible();
      await expect(it.getByRole("img", { name: /^Deutsch \d von 5$/ })).toBeVisible();
    }
  }
  if (isPhone(page)) await column(page, "Rollenspiel");
  await page.locator('[data-spot="best-models"]').scrollIntoViewIfNeeded();
  await shot(page, "empfehlung", info.project.name);

  // "Nur unzensierte" asks the server again and shows its other answer.
  const unc = await catalog(true);
  const asked = page.waitForRequest((r) => r.url().includes("/api/models/catalog?uncensored=1"));
  const sw = page.getByRole("switch", { name: /Nur unzensierte/ });
  await sw.check();
  await asked;
  const uncensoredOk = (id: string) => unc.models.find((m) => m.id === id)!.uncensored >= 1;
  for (const [key, name] of COLUMNS) {
    const col = await column(page, name);
    const items = col.getByRole("listitem");
    await expect(items).toHaveCount(unc.best![key].length);
    for (const [i, p] of unc.best![key].entries()) await expect(items.nth(i)).toHaveAccessibleName(nameOf(p.id));
    expect(unc.best![key].every((p) => uncensoredOk(p.id))).toBe(true);
  }
  // Whether the picks differ depends on the PC (with 24 GB the best ones already are uncensored);
  // the page shows the server's second answer either way, and a model that refuses is never in it.
  expect(cat.best!.roleplay.concat(cat.best!.tempo, cat.best!.context).every((p) => uncensoredOk(p.id)) || JSON.stringify(unc.best) !== JSON.stringify(cat.best)).toBe(true);
  // The switch is kept in this browser.
  await page.reload();
  await expect(page.getByRole("switch", { name: /Nur unzensierte/ })).toBeChecked();
  await page.getByRole("switch", { name: /Nur unzensierte/ }).uncheck();
  await expect((await column(page, "Rollenspiel")).getByRole("listitem").first()).toHaveAccessibleName(nameOf(cat.best!.roleplay[0].id));
});

/** The filters: the column beside the cards, or the sheet on a phone (opened here). */
async function filters(page: Page): Promise<Locator> {
  if (!isPhone(page)) return page.locator('[data-spot="suggestions"]').getByRole("complementary", { name: "Filter" });
  const sheet = page.getByRole("dialog", { name: "Filter" });
  if (!(await sheet.isVisible())) await page.locator('[data-spot="suggestions"]').getByRole("button", { name: /^Filter( \d+)?$/ }).click();
  await expect(sheet).toBeVisible();
  return sheet;
}

async function resetFilters(page: Page) {
  if (isPhone(page)) {
    const sheet = page.getByRole("dialog", { name: "Filter" });
    await sheet.getByRole("button", { name: "Zurücksetzen" }).click();
    await sheet.getByRole("button", { name: /Treffer zeigen$/ }).click();
    await expect(sheet).toBeHidden();
  } else await page.locator('[data-spot="suggestions"]').getByRole("button", { name: "Filter zurücksetzen" }).click();
}

test("Vorschläge: jeder Filter grenzt ein, „zurücksetzen“ hebt auf, Sortierung", async ({ page, open }, info) => {
  const data = process.env.RP_E2E_DATA;
  // One installed file, so "Installiert" / "Nicht installiert" have something to tell apart.
  const app = data ? (await api<{ folders: { app: { path: string } } }>("GET", "/models/local")).folders.app.path : null;
  const target = cat.models.find((m) => m.repo !== CATALOG_REPO && m.quants.small)!;
  const file = app ? join(app, "e2e-sug", target.quants.small!.file) : null;
  if (file) writeMiniGguf(file, { name: "E2E Vorschlag", bytes: 4096 });
  try {
    const now = await catalog();
    await open("/settings/suggestions");
    const panel = page.locator('[data-spot="suggestions"]');
    // The cards themselves, not the source links listed inside them.
    const cards = panel.getByRole("list", { name: "Vorschläge" }).locator(":scope > li");
    const count = panel.locator(".sug-count");
    const all = cat.models.length;
    await expect(count).toHaveText(`${all} von ${all}`);
    await expect(cards).toHaveCount(all);

    const p = (m: Model) => m.params_total_b ?? 0;
    const installed = (m: Model) => Object.values(now.models.find((x) => x.id === m.id)!.quants).some((q) => q?.installed);
    type Step = [string, (f: Locator) => Promise<void>, (m: Model) => boolean];
    const chip = (group: string, name: string) => async (f: Locator) => f.getByRole("group", { name: group }).getByRole("button", { name, exact: true }).click();
    const steps: Step[] = [
      ["Typ MoE", chip("Typ", "MoE"), (m) => m.moe],
      ["Typ Dicht", chip("Typ", "Dicht"), (m) => !m.moe],
      ["Familie Gemma", chip("Familie", "Gemma"), (m) => m.base === "gemma"],
      ["Parameter ab 24", async (f) => f.getByRole("slider", { name: "Parameter ab" }).fill("5"), (m) => p(m) >= 24],
      ["Parameter bis 12", async (f) => f.getByRole("slider", { name: "Parameter bis" }).fill("3"), (m) => p(m) <= 12],
      ["Kontext ab 256k", chip("Kontext mindestens", "256k"), (m) => (m.context_max ?? 0) >= 262_144],
      ["Rollenspiel ab 4", chip("Rollenspiel mindestens", "ab 4"), (m) => m.rp >= 4],
      ["Deutsch ab 4", chip("Deutsch mindestens", "ab 4"), (m) => m.de >= 4],
      ["Zensur frei", chip("Zensur", "Frei"), (m) => m.uncensored === 2],
      ["Zensur meist offen", chip("Zensur", "Meist offen"), (m) => m.uncensored >= 1],
      ["Lizenz Apache", chip("Lizenz", "Apache 2.0"), (m) => m.license === "apache-2.0"],
      ["Stufe Klein", chip("Stufe", "Klein"), (m) => !!m.quants.small && !m.quants.small.missing],
    ];
    // "Passt auf meinen PC" keeps what the server's pill calls fitting: with a graphics card first
    // what runs all on it, with the second level also what puts a part into the RAM; without a
    // usable card whatever runs in the RAM (its pill has no VRAM part).
    const onPc = (wide: boolean) => (m: Model) =>
      Object.values(m.quants).some((q) => !!q && !q.missing && (q.fit === "fits" || (q.fit === "cpu_help" && (wide || !(q as { gpu_gb?: number | null }).gpu_gb))));
    if (cat.models.some((m) => Object.values(m.quants).some((q) => q?.fit))) {
      const f = await filters(page);
      await f.getByRole("switch", { name: /Passt auf meinen PC/ }).check();
      await expect(count, "passt").toHaveText(`${cat.models.filter(onPc(false)).length} von ${all}`);
      const wide = f.getByRole("group", { name: "Wie es passen soll" }).getByRole("button", { name: "Auch mit Arbeitsspeicher, langsamer" });
      if (await wide.count()) {
        await wide.click();
        await expect(count, "passt mit Arbeitsspeicher").toHaveText(`${cat.models.filter(onPc(true)).length} von ${all}`);
      }
      await resetFilters(page);
      await expect(count, "passt reset").toHaveText(`${all} von ${all}`);
    }
    if (file) {
      steps.push(["Installiert", chip("Installiert", "Installiert"), installed]);
      steps.push(["Nicht installiert", chip("Installiert", "Nicht installiert"), (m) => !installed(m)]);
    }
    for (const [what, act, keep] of steps) {
      const f = await filters(page);
      await act(f);
      const n = cat.models.filter(keep).length;
      expect(n, `${what} narrows the catalog`).toBeLessThan(all);
      await expect(count, what).toHaveText(`${n} von ${all}`);
      await expect(cards, what).toHaveCount(n);
      await resetFilters(page);
      await expect(count, `${what} reset`).toHaveText(`${all} von ${all}`);
    }

    // Two filters together, with the review screenshots: the filters open, then the list they leave.
    const f = await filters(page);
    await chip("Rollenspiel mindestens", "ab 4")(f);
    await chip("Deutsch mindestens", "ab 4")(f);
    const both = cat.models.filter((m) => m.rp >= 4 && m.de >= 4);
    await expect(count).toHaveText(`${both.length} von ${all}`);
    await (isPhone(page) ? f : panel).scrollIntoViewIfNeeded();
    if (!isPhone(page)) await panel.evaluate((el) => el.scrollIntoView({ block: "start" }));
    await shot(page, "filter-offen", info.project.name);
    if (isPhone(page)) await f.getByRole("button", { name: /Treffer zeigen$/ }).click();
    await expect(panel.getByRole("button", { name: /^Filter( \d+)?$/ })).toContainText("2");

    // Sort by roleplay: highest rating first, ties by quality, then name.
    await panel.getByLabel("Sortieren").selectOption("rp");
    const byRp = [...both].sort((a, b) => b.rp - a.rp || b.q - a.q || a.name.localeCompare(b.name, "de"));
    for (const [i, m] of byRp.slice(0, 3).entries()) await expect(cards.nth(i)).toHaveAccessibleName(m.name);
    await cards.first().scrollIntoViewIfNeeded();
    await shot(page, "gefiltert", info.project.name);

    // The choice is kept in this browser; a filter that leaves nothing keeps the frame and says so.
    await page.reload();
    await expect(count).toHaveText(`${both.length} von ${all}`);
    await expect(panel.getByLabel("Sortieren")).toHaveValue("rp");
    const g = await filters(page);
    await chip("Kontext mindestens", "256k")(g);
    await chip("Typ", "MoE")(g);
    if (!cat.models.some((m) => m.rp >= 4 && m.de >= 4 && m.moe && (m.context_max ?? 0) >= 262_144)) {
      await expect(panel.getByText("Kein Vorschlag passt zu diesen Filtern.")).toBeAttached();
    }
    await resetFilters(page);
    await expect(count).toHaveText(`${all} von ${all}`);
  } finally {
    if (file) rmSync(join(app!, "e2e-sug"), { recursive: true, force: true });
  }
});

test("Vorschläge: ein installiertes Modell zeigt „Benutzen“ und wird damit aktiv", async ({ page, open }, info) => {
  const data = process.env.RP_E2E_DATA;
  test.skip(!data || !cat.best, "RP_E2E_DATA is not set or the PC could not be read");
  test.setTimeout(360_000); // the CPU model loads again afterwards
  const profiles = async () => (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles;
  const original = (await profiles()).find((x) => x.active)!;
  const app = (await api<{ folders: { app: { path: string } } }>("GET", "/models/local")).folders.app.path;
  // The file of the first roleplay pick, in its tier, as a tiny stand-in on disk.
  const pick = cat.best!.roleplay[0];
  const model = cat.models.find((m) => m.id === pick.id)!;
  const file = join(app, "e2e-sug", model.quants[pick.tier]!.file);
  writeMiniGguf(file, { name: "E2E Benutzen", bytes: 4096 });
  try {
    await open("/settings/suggestions");
    const best = (await column(page, "Rollenspiel")).getByRole("listitem", { name: model.name, exact: true });
    await expect(best.getByText("Installiert", { exact: true })).toBeVisible();
    await expect(best.getByRole("button", { name: `${model.name} benutzen, ${TIER_NAME[pick.tier]}` })).toBeVisible();
    await expect(best.getByRole("button", { name: /herunterladen/ })).toHaveCount(0);

    const card = page.locator('[data-spot="suggestions"]').getByRole("listitem", { name: model.name, exact: true });
    await card.scrollIntoViewIfNeeded();
    await card.getByRole("radio", { name: new RegExp(`^.?${TIER_NAME[pick.tier]}`) }).click();
    const used = page.waitForResponse((r) => r.url().includes("/api/models/local/use"));
    await card.getByRole("button", { name: `${model.name} benutzen, ${TIER_NAME[pick.tier]}` }).click();
    const answer = (await (await used).json()) as { created: boolean; placement: Placement | null };
    await expect(card.getByRole("status")).toContainText(/Wird geladen|Als aktiv gespeichert|Läuft schon/);
    // A new profile on a known PC: the card says, in words, how the model starts.
    expect(answer.created).toBe(true);
    if (answer.placement) await expect(card.locator(".r1-placed")).toHaveText(placementText(answer.placement));
    else await expect(card.locator(".r1-placed")).toHaveCount(0);
    if (SHOTS) {
      await card.locator(".sg-act").scrollIntoViewIfNeeded();
      await shot(page, "r1-benutzen", info.project.name);
    }
    await expect.poll(async () => (await profiles()).find((x) => x.active)?.model_file.toLowerCase()).toBe(file.toLowerCase());
  } finally {
    // Back to the profile the suite started with; the test profile and file go.
    const now = await profiles();
    if (now.find((x) => x.active)?.id !== original.id) await api("POST", `/models/profiles/${original.id}/activate`);
    for (const p of now) if (p.model_file.toLowerCase() === file.toLowerCase() && p.id !== original.id) await api("DELETE", `/models/profiles/${p.id}`);
    rmSync(join(app, "e2e-sug"), { recursive: true, force: true });
    const managed = (await api<{ managed: boolean }>("GET", "/engine")).managed;
    if (managed) {
      await expect
        .poll(async () => (await api<{ status: string }>("GET", "/engine")).status, { timeout: 300_000, intervals: [500, 1000, 2000] })
        .toBe("ready");
    }
  }
});

test("Vorschläge: Karte lädt herunter, Badge zählt, Pause und Fortsetzen", async ({ page, open }, info) => {
  test.skip(!viaStandIn, "the test server does not point at the Hugging Face stand-in (RP_HF_BASE)");
  await clean();
  await open("/settings/suggestions");
  const entry = cat.models.find((m) => m.repo === CATALOG_REPO)!;
  const card = page.locator('[data-spot="suggestions"]').getByRole("listitem", { name: entry.name, exact: true });
  await card.scrollIntoViewIfNeeded();
  await card.getByRole("radio", { name: /^.?Klein/ }).click();
  await card.getByRole("button", { name: /herunterladen, Klein/ }).click();

  const badge = isPhone(page) ? page.locator("header.top .dl-badge") : page.getByRole("complementary").getByRole("link", { name: /Modelle finden/ });
  await expect(badge).toContainText("1");
  const row = page.getByRole("listitem", { name: /^Download Huihui-Qwen3\.5-4B-abliterated\.IQ4_XS/ });
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "Pausieren" }).click();
  await expect(row).toContainText("Pausiert");
  await row.scrollIntoViewIfNeeded();
  await shot(page, "download-pausiert", info.project.name);
  await row.getByRole("button", { name: "Fortsetzen" }).click();
  await expect(row).toContainText("Fertig", { timeout: 30_000 });
  await expect(isPhone(page) ? badge : badge.locator(".dl-count")).toHaveCount(0);
  await expect(card.getByText("Installiert")).toBeVisible();
  await clean();
});

test("Vorschläge: Hugging-Face-Link mit Größenwahl, Abbrechen löscht", async ({ page, open }, info) => {
  test.skip(!viaStandIn, "the test server does not point at the Hugging Face stand-in (RP_HF_BASE)");
  await clean();
  await open("/settings/suggestions");
  const box = page.locator('[data-spot="hf-link"]');
  await box.getByLabel("Hugging-Face-Link").fill(`https://huggingface.co/${LINK_REPO}`);
  await box.getByRole("button", { name: "Dateien zeigen" }).click();
  const tiers = box.getByRole("radiogroup", { name: "Größe" });
  await expect(tiers.getByRole("radio", { name: /Ausgewogen/ })).toHaveAttribute("aria-checked", "true");
  await tiers.getByRole("radio", { name: /Groß/ }).click();
  await box.getByText("Bildverständnis dazu laden").click();
  await expect(box.getByRole("button", { name: /Herunterladen · 6 MB/ })).toBeVisible();
  await box.scrollIntoViewIfNeeded();
  await shot(page, "hf-link", info.project.name);
  await box.getByRole("button", { name: /Herunterladen/ }).click();
  await expect(box.getByText("Download gestartet.")).toBeVisible();

  const row = page.getByRole("listitem", { name: /^Download Tiny-Q6_K/ });
  await expect(row).toContainText("2 Dateien");
  await row.getByRole("button", { name: /abbrechen/ }).click();
  await row.getByRole("button", { name: "Abbrechen und löschen" }).click();
  await expect(row).toHaveCount(0);
  await expect.poll(async () => (await downloads()).length).toBe(0);
});

test("Mehr finden: Live-Suche zeigt nicht kuratierte Treffer mit Größenwahl", async ({ page, open }, info) => {
  test.skip(!viaStandIn, "the test server does not point at the Hugging Face stand-in (RP_HF_BASE)");
  await open("/settings/suggestions");
  const box = page.locator('[data-spot="find-more"]');
  await box.getByLabel("Modell suchen").fill("tiny");
  await box.getByRole("button", { name: "Suchen" }).click();
  const hit = box.getByRole("list", { name: "Treffer" }).getByRole("listitem").first();
  await expect(hit).toContainText("nicht kuratiert");
  await hit.getByRole("button", { name: "Dateien zeigen" }).click();
  await expect(hit.getByRole("radiogroup", { name: "Größe" })).toBeVisible();
  await box.scrollIntoViewIfNeeded();
  await shot(page, "mehr-finden", info.project.name);
});

test("Modell-Tab: „Neue Modelle finden“ führt zu Vorschlägen und dem Download per Link", async ({ page, open }, info) => {
  await open("/settings/model");
  await shot(page, "modell-tab", info.project.name);
  await page.getByRole("link", { name: /Neue Modelle finden/ }).click();
  await expect(page).toHaveURL(/#\/settings\/suggestions$/);
  await expect(page.locator('[data-spot="hf-link"]').getByLabel("Hugging-Face-Link")).toBeVisible();
});

test("Strg K: „Modelle finden“ öffnet den Tab Vorschläge", async ({ page, open }) => {
  await open("/chars");
  await page.keyboard.press("Control+k");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  await dialog.getByRole("combobox").fill("modell herunterladen");
  await expect(dialog.getByRole("option").first()).toContainText(/Modelle finden|Hugging-Face-Link|Downloads/);
  await dialog.getByRole("combobox").fill("hugging face link");
  await expect(dialog.getByRole("option").first()).toContainText("Hugging-Face-Link einfügen");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/settings\/suggestions$/);
  await expect(page.getByLabel("Hugging-Face-Link")).toBeFocused();
});

test("Strg K: „Bestes Modell“, „Filter“ und „Nur unzensierte“", async ({ page, open }) => {
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  const ask = async (text: string, label: string | RegExp) => {
    await page.keyboard.press("Control+k");
    await dialog.getByRole("combobox").fill(text);
    await expect(dialog.getByRole("option").first()).toContainText(label);
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/#\/settings\/suggestions$/);
  };
  await open("/chars");
  await ask("bestes modell", "Bestes Modell für deinen PC");
  await expect(page.locator('[data-spot="best-models"]')).toBeInViewport();

  await open("/chars");
  await ask("filter", "Modell-Filter");
  const f = isPhone(page) ? page.getByRole("dialog", { name: "Filter" }) : page.getByRole("complementary", { name: "Filter" });
  await expect(f).toBeVisible();
  await expect(f.getByRole("switch", { name: /Passt auf meinen PC|Passt in den Grafikspeicher/ })).toBeFocused();

  await open("/chars");
  await ask("nur unzensierte", "Nur unzensierte");
  await expect(page.getByRole("switch", { name: /Nur unzensierte/ })).toBeFocused();
});
