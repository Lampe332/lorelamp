// Y1: "Was willst du mit dem Modell machen?" (tab "Vorschläge") and "Beste Einstellung für deinen
// Zweck" (tab "Modell"). A sentence becomes chips, the chips can be corrected, and the answer and the
// catalog stand in the server's order for the purpose on a simulated PC (stored PC scan, as in
// best-pc.spec.ts). On the model tab the purpose picks the measured benchmark row and derives sampler
// values; "Setzen" writes both into the profile. Everything the tests change is put back: the PC
// scan, the purpose, the profile and its stored benchmark results (`RP_E2E_DATA`, never the owner's).
// `RP_E2E_SHOTS` (optional): review screenshots go there.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { Page } from "@playwright/test";
import type { Advice } from "../src/models/purposeApi";
import type { Catalog, UsePurpose } from "../src/models/types";
import { NEED_NAME, reasonLine } from "../src/models/words";
import { api } from "./api";
import { dataDir } from "./db";
import { expect, isPhone, test } from "./fixtures";

const GiB = 2 ** 30;
const SCANS = {
  "nvidia-8": { gpus: [{ name: "GeForce RTX 4060", vendor: "nvidia", vram_total: 8 * GiB }], ram_total: 32 * GiB, backend: { suggested: "cuda" } },
  "amd-24": { gpus: [{ name: "Radeon RX 7900 XTX", vendor: "amd", vram_total: 24 * GiB }], ram_total: 32 * GiB, backend: { suggested: "rocm" } },
  "cpu-16": { gpus: [], ram_total: 16 * GiB, backend: { suggested: "cpu" } },
} as const;
const SHOTS = process.env.RP_E2E_SHOTS;
const OWNER = "Ich will richtig lange Chats mit 3–4 User-Charakteren";

/** A settings row of the test database: read (no value), write, or remove (null). */
function setting(key: string, value?: string | null): string | null {
  const db = new DatabaseSync(join(dataDir, "rp-app.db"));
  try {
    db.exec("PRAGMA busy_timeout = 5000");
    if (value === undefined) return (db.prepare("SELECT value FROM settings WHERE key = ?").get(key) as { value: string } | undefined)?.value ?? null;
    if (value === null) db.prepare("DELETE FROM settings WHERE key = ?").run(key);
    else db.prepare("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(key, value);
    return value;
  } finally {
    db.close();
  }
}

const putPurpose = (text: string, needs?: string[]) => api<UsePurpose>("PUT", "/models/purpose", needs ? { text, needs } : { text });
const pressed = (page: Page, scope: string) => page.locator(scope).locator('.pu-chip[aria-pressed="true"]');

async function shot(page: Page, name: string, project: string) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(SHOTS, `${name}-${project}.png`) });
}

test.describe("Zweck in den Vorschlägen", () => {
  test.skip(!dataDir, "needs RP_E2E_DATA (the test server's --data-dir)");
  let scan: string | null = null;
  test.beforeEach(async () => {
    scan = setting("system_scan");
    await putPurpose("");
  });
  test.afterEach(async () => {
    setting("system_scan", scan);
    await putPurpose("");
  });

  for (const pc of ["nvidia-8", "amd-24", "cpu-16"] as const) {
    test(`${pc}: ein Satz wird zu Chips, Antwort und Vorschläge ordnen sich danach`, async ({ page, open }, info) => {
      setting("system_scan", JSON.stringify(SCANS[pc]));
      await open("/settings/suggestions");
      const row = page.locator('[data-spot="model-purpose"]');
      const field = row.getByRole("textbox", { name: "Dein Zweck" });
      const cards = page.locator('[data-spot="suggestions"]').getByRole("list", { name: "Vorschläge" }).locator(":scope > li .sg-name");
      await expect(field).toHaveValue("");
      await expect(row.locator(".pu-chip")).toHaveCount(12);
      await expect(pressed(page, '[data-spot="model-purpose"]')).toHaveCount(0);
      await expect(row.getByRole("region", { name: "Für deinen Zweck" })).toHaveCount(0);
      await expect(cards.first()).toBeVisible();
      const before = await cards.allTextContents();

      // The owner's sentence: two chips, read from the text.
      await field.fill(OWNER);
      await expect(pressed(page, '[data-spot="model-purpose"]')).toHaveText([NEED_NAME.long_chats, NEED_NAME.group]);
      await expect(row.getByRole("button", { name: NEED_NAME.long_chats })).toHaveAttribute("title", "Erkannt an „lange chats“");
      await expect(row.locator(".pu-hint")).toContainText("Aus deinem Satz gelesen");
      const answer = row.getByRole("region", { name: "Für deinen Zweck" });
      await expect(answer).toBeVisible();
      await expect(answer).not.toHaveAttribute("aria-busy", "true");

      // The page shows the server's answer for this PC: the same models in the same order, each with its reason.
      const cat = await api<Catalog>("GET", "/models/catalog");
      expect(cat.pc?.source).toBe("scan");
      expect(cat.purpose?.needs).toEqual(["long_chats", "group"]);
      const ranked = cat.for_purpose!.ranked;
      const name = (id: string) => cat.models.find((m) => m.id === id)!.name;
      const top = cat.for_purpose!.top.map((id) => ranked.find((s) => s.id === id)!);
      expect(top.length).toBeGreaterThan(0);
      const picks = answer.getByRole("listitem");
      await expect(picks.locator(".pu-head b")).toHaveText(top.map((s) => name(s.id)));
      await expect(picks.locator(".pu-why")).toHaveText(top.map((s) => reasonLine(s)));
      await expect(picks.first().locator(".pu-why")).toContainText(/Kontext|Rollenspiel/);

      // The catalog below stands in that order too (what does not run on this PC comes last), with the reason at the card.
      const order = ranked.map((s) => name(s.id));
      await expect(cards).toHaveCount(cat.models.length);
      const after = await cards.allTextContents();
      expect(after.slice(0, order.length)).toEqual(order);
      expect(after).not.toEqual(before);
      await expect(page.locator('[data-spot="suggestions"] [data-purpose-why]')).toHaveCount(order.length);
      await shot(page, `y1-purpose-${pc}`, info.project.name);

      // A chip corrected by hand counts, and the answer follows.
      await row.getByRole("button", { name: NEED_NAME.fast }).click();
      await expect(pressed(page, '[data-spot="model-purpose"]')).toHaveText([NEED_NAME.long_chats, NEED_NAME.group, NEED_NAME.fast]);
      await expect(row.locator(".pu-hint")).toContainText("Von dir angepasst");
      await expect.poll(async () => (await api<UsePurpose>("GET", "/models/purpose")).needs).toEqual(["long_chats", "group", "fast"]);
      const fast = await api<Catalog>("GET", "/models/catalog");
      await expect(answer.getByRole("listitem").locator(".pu-head b")).toHaveText(fast.for_purpose!.top.map((id) => fast.models.find((m) => m.id === id)!.name));

      // A new sentence is read again (English this time); the hand-made choice is gone.
      await field.fill("fast replies in English, uncensored");
      await expect(pressed(page, '[data-spot="model-purpose"]')).toHaveText([NEED_NAME.english, NEED_NAME.fast, NEED_NAME.uncensored]);
      await expect.poll(async () => (await api<Catalog>("GET", "/models/catalog")).purpose?.needs).toEqual(["english", "fast", "uncensored"]);
      const open_only = await api<Catalog>("GET", "/models/catalog");
      const refusing = open_only.models.filter((m) => m.uncensored === 0).map((m) => m.name);
      await expect(answer.getByRole("listitem").locator(".pu-head b")).toHaveText(open_only.for_purpose!.top.map((id) => open_only.models.find((m) => m.id === id)!.name));
      for (const n of await answer.getByRole("listitem").locator(".pu-head b").allTextContents()) expect(refusing).not.toContain(n);

      // The purpose is kept: it is there after a reload.
      await page.reload();
      await expect(row.getByRole("textbox", { name: "Dein Zweck" })).toHaveValue("fast replies in English, uncensored");
      await expect(pressed(page, '[data-spot="model-purpose"]')).toHaveCount(3);

      // Emptied: no answer, the catalog in its own order again.
      await row.getByRole("button", { name: "Zweck leeren" }).click();
      await expect(pressed(page, '[data-spot="model-purpose"]')).toHaveCount(0);
      await expect(row.getByRole("region", { name: "Für deinen Zweck" })).toHaveCount(0);
      await expect(page.locator('[data-spot="suggestions"] [data-purpose-why]')).toHaveCount(0);
      await expect.poll(async () => await cards.allTextContents()).toEqual(before);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    });
  }

  test("Strg K findet den Zweck", async ({ page, open }) => {
    test.skip(isPhone(page), "the palette's shortcut is a desktop one");
    await open("/");
    await page.keyboard.press("Control+k");
    const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
    await dialog.getByRole("combobox").fill("zweck modell");
    await dialog.getByRole("option").filter({ hasText: "Was willst du mit dem Modell machen?" }).first().click();
    await expect(page).toHaveURL(/#\/settings\/suggestions/);
    await expect(page.locator('[data-spot="model-purpose"]')).toBeInViewport();
    await page.keyboard.press("Control+k");
    await dialog.getByRole("combobox").fill("einstellung zweck");
    await dialog.getByRole("option").filter({ hasText: "Beste Einstellung für deinen Zweck" }).first().click();
    await expect(page).toHaveURL(/#\/settings\/model/);
    await expect(page.locator('[data-spot="purpose-advice"]')).toBeInViewport();
  });
});

type Profile = { id: number; active: boolean; load_args: string[]; sampler: Record<string, number>; measured: boolean; model_file: string; engine_id: number; created_at: number };

test.describe("Beste Einstellung für deinen Zweck", () => {
  test.skip(!dataDir, "needs RP_E2E_DATA (the test server's --data-dir)");

  test("Zweck eingeben, „Setzen“ schreibt Startoptionen und Sampler ins Profil", async ({ page, open }, info) => {
    test.setTimeout(240_000);
    const profiles = (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles;
    const p = profiles.find((x) => x.active);
    test.skip(!p, "needs an active model profile (test server with --manage-engine)");
    const keys = [`bench_result_${p!.id}`, `bench_results_${p!.id}`];
    const saved = keys.map((k) => setting(k));
    await putPurpose("");
    // A benchmark result of this profile, with the picks `rp_core::bench::recommend` gives for these
    // rows: 4k keeps 90 % of the fastest ("Ausgewogen"), 8k writes 16 tokens/s (the only row from
    // 8k up that is fast enough: "Tempo" and "Kontext"), 16k is too slow to read along.
    const row = (ctx: number, tg: number) => ({ ctx, kv: "q8_0", n_cpu_moe: 0, pp: 400, tg, overflow_mib: 0, outcome: "ok", load_s: 3, pp_runs: [400, 400, 400], tg_runs: [tg, tg, tg], spread: 0.01, uncertain: false, limit: null });
    const result = {
      at: Math.floor(Date.now() / 1000) - 60,
      duration_s: 61,
      gpu: null,
      prompt_tokens: 256,
      gen_tokens: 16,
      rows: [row(2048, 40), row(4096, 36), row(8192, 16), row(16384, 9)],
      picks: [
        { purpose: "speed", row: 2 },
        { purpose: "balanced", row: 1 },
        { purpose: "context", row: 2 },
      ],
      conditions: { ram_available_mib: 20000, vram_free_mib: null },
      measured_with: { model_file: p!.model_file, engine_id: p!.engine_id, profile_created_at: p!.created_at },
    };
    setting(keys[0], JSON.stringify(result));
    setting(keys[1], JSON.stringify([result]));
    try {
      await open("/settings/model");
      await page.getByRole("button", { name: "Beste Einstellung finden" }).click();
      const card = page.locator('[data-spot="purpose-advice"]');
      await card.scrollIntoViewIfNeeded();
      const field = card.getByRole("textbox", { name: "Wofür nutzt du das Modell?" });
      await expect(field).toHaveValue("");
      // Without a purpose: the balanced row of the measurement, nothing derived.
      const measured = card.getByRole("group", { name: "Gemessen" });
      const derived = card.getByRole("group", { name: "Abgeleitet" });
      await expect(measured).toContainText("4k Kontext");
      // Derived without a purpose: only what the model's author recommends, when the suite's model
      // file is one of the catalog (Gemma 4 E2B is: temperature 1, top-p 0.95, top-k 64).
      const author = (await api<Advice>("GET", `/models/profiles/${p!.id}/advice`)).derived.filter((d) => d.why === "author");
      const byAuthor = card.getByRole("group", { name: "Vom Modell-Autor empfohlen" });
      await expect(derived).toContainText("Noch kein Zweck angegeben");
      await expect(byAuthor).toHaveCount(author.length ? 1 : 0);
      await expect(byAuthor.locator("[data-field]")).toHaveCount(author.length);
      if (author.length) await expect(byAuthor).toContainText("Modellkarte");

      await field.fill(OWNER);
      await expect(pressed(page, '[data-spot="purpose-advice"]')).toHaveText([NEED_NAME.long_chats, NEED_NAME.group]);
      // Measured: the largest context that still reads fluently. Derived: reply room for four
      // characters and DRY against the repetition of long chats.
      await expect(measured).toContainText("8k Kontext");
      await expect(measured).toContainText("Kontext: der größte Kontext, der noch flüssig schreibt, 16 Tokens/s");
      await expect(measured).toContainText("KV-Cache q8_0");
      await expect(derived.locator('[data-field="n_predict"]')).toContainText("Antwortlänge 1.100 Tokens");
      await expect(derived.locator('[data-field="n_predict"]')).toContainText("Platz für 4 Figuren");
      await expect(derived.locator('[data-field="dry_multiplier"]')).toContainText("Wiederholungsschutz (DRY) 0,8");
      await expect(derived.locator('[data-field="dry_multiplier"]')).toContainText("Lange Chats wiederholen sich sonst");
      await expect(derived.locator('[data-field="dry_allowed_length"]')).toContainText("3 Tokens am Stück");
      await shot(page, "y1-advice", info.project.name);

      await card.getByRole("button", { name: "Einstellung für deinen Zweck setzen" }).click();
      await expect(card.getByRole("status").filter({ hasText: "Gesetzt" })).toBeVisible();
      // The profile now has the measured start options and the derived sampler; the rest stayed.
      const now = (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles.find((x) => x.id === p!.id)!;
      const arg = (flag: string) => now.load_args[now.load_args.indexOf(flag) + 1];
      expect([arg("-c"), arg("-ctk"), arg("-ctv")]).toEqual(["8192", "q8_0", "q8_0"]);
      expect(now.load_args).not.toContain("--n-cpu-moe");
      for (const flag of ["-ngl", "-np"]) expect(now.load_args[now.load_args.indexOf(flag) + 1]).toBe(p!.load_args[p!.load_args.indexOf(flag) + 1]);
      expect(now.sampler).toMatchObject({ n_predict: 1100, dry_multiplier: 0.8, dry_base: 1.75, dry_allowed_length: 3 });
      for (const d of author) expect(now.sampler[d.field]).toBe(d.value);
      const proposed = ["n_predict", ...author.map((d) => d.field as string)];
      for (const [k, v] of Object.entries(p!.sampler)) if (!proposed.includes(k)) expect(now.sampler[k]).toBe(v);
      expect(now.measured).toBe(true);
      // The purpose is kept in the settings table, for both places.
      expect(JSON.parse(setting("use_purpose")!)).toMatchObject({ text: OWNER, needs: ["long_chats", "group"], figures: 4 });
      // Every proposed value now says that it is set: context, KV cache, the four derived ones and the author's.
      await expect(card.locator(".pu-facts li.set")).toHaveCount(6 + author.length);
      // The list reads the same as before "Setzen" (nothing vanishes once it is set).
      await expect(derived.locator("[data-field]")).toHaveCount(4);
      await expect(card.locator(".pu-is-set").first()).toHaveText(/^Gesetzt\./);
      await shot(page, "y1-advice-set", info.project.name);

      // "Fast" changes the measured row and the reply length; nothing is written until "Setzen".
      await card.getByRole("button", { name: NEED_NAME.fast }).click();
      await expect(measured).toContainText("4k Kontext");
      await expect(derived.locator('[data-field="n_predict"]')).toContainText("Antwortlänge 650 Tokens");
      await expect(card.getByRole("button", { name: "Einstellung für deinen Zweck setzen" })).toBeVisible();
      const untouched = (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles.find((x) => x.id === p!.id)!;
      expect(untouched.sampler.n_predict).toBe(1100);
    } finally {
      // Back to the suite's profile, its results and no purpose; the model starts with its own options again.
      await api("PUT", `/models/profiles/${p!.id}`, { load_args: p!.load_args, sampler: p!.sampler, measured: p!.measured });
      keys.forEach((k, i) => setting(k, saved[i]));
      await putPurpose("");
      await expect.poll(async () => (await api<{ status: string; model: { ctx: number } | null }>("GET", "/engine")).status, { timeout: 180_000 }).toBe("ready");
    }
  });
});
