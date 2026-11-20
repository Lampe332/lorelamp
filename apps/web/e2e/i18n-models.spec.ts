// The model settings in English, Spanish and Russian hold no German (dictionary `models`, see
// docs/i18n.md): the tab Modell with "Meine Modelle" and "Beste Einstellung finden" (the purpose
// advice and a stored benchmark result), the tab Experte (profiles, a new profile, the editor, port,
// model folders, the PC scan, the engines and their download), the tab Vorschläge (purpose with its
// answer, "Bestes Modell", the catalog with its filters, the phone's filter sheet), a pasted Hugging
// Face link with its sizes, "Mehr finden" with a result, and a download in the list — on desktop,
// phone and tablet. Plus review screenshots at 1280 and 375 px in English and Russian
// (`RP_E2E_SHOTS`), each checked for horizontal overflow.
//
// Needs the model server managed by the test server (`--manage-engine`, for the benchmark result: a
// real run when none is stored) and, for the link, the search and the download, the local Hugging
// Face stand-in (`RP_HF_BASE`, see README; those screens skip themselves without it). The stored
// purpose is set to an English sentence and put back afterwards; downloads and their files are
// removed again.

import { mkdirSync, rmSync } from "node:fs";
import type { Server } from "node:http";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { api } from "./api";
import { expect, test } from "./fixtures";
import { HF_PORT, LINK_REPO, startHfStandIn } from "./hf-standin";
import { leftoverTests, type Screen } from "./leftovers";

type Need = string;
type Purpose = { text: string; needs: Need[]; parsed: Need[] };
type Profile = { id: number; active: boolean };
type Engine = { status: string; managed: boolean; bench_profile_id?: number | null };

const SHOTS = process.env.RP_E2E_SHOTS;
const engine = () => api<Engine>("GET", "/engine");
const active = async () => (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles.find((p) => p.active) ?? null;

let hf: Server | null = null;
let viaStandIn = false;
let purposeBefore: Purpose | null = null;

/** Every download gone and the stand-in's files out of the model folder. */
async function clean() {
  for (const d of (await api<{ downloads: { id: string }[] }>("GET", "/models/downloads")).downloads) await api("DELETE", `/models/downloads/${d.id}`);
  const data = process.env.RP_E2E_DATA;
  // Windows: the server may still hold the file it just stopped downloading for a moment.
  if (data) rmSync(join(data, "models", ...LINK_REPO.split("/")), { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
}

test.beforeAll(async () => {
  test.setTimeout(300_000);
  // The PC scan the tab Experte shows (the real PC; scanning changes nothing).
  if (!(await api<{ scan: unknown }>("GET", "/system/scan")).scan) await api("POST", "/system/scan");
  // A stored benchmark result for the active profile, so its table and cards are on screen.
  const e = await engine();
  const p = await active();
  if (e.managed && p) {
    await expect.poll(async () => (await engine()).status === "ready" && !(await engine()).bench_profile_id, { timeout: 180_000, intervals: [1000, 2000] }).toBe(true);
    const bench = () => api<{ running: unknown; result: unknown }>("GET", `/models/profiles/${p.id}/benchmark`);
    // A run that starts while the model server is still going down measures nothing: a second try.
    for (let tries = 0; tries < 2 && !(await bench()).result; tries++) {
      await api("POST", `/models/profiles/${p.id}/benchmark`, {});
      await expect.poll(async () => !(await bench()).running, { timeout: 200_000, intervals: [2000] }).toBe(true);
      await expect.poll(async () => (await engine()).status, { timeout: 120_000 }).toBe("ready");
    }
    expect((await bench()).result, "a stored benchmark result").toBeTruthy();
  }
  // A purpose in English: the chips, the answer and every card's reason show.
  purposeBefore = await api<Purpose>("GET", "/models/purpose");
  await api("PUT", "/models/purpose", { text: "Really long chats with three characters and fast replies, in English" });
  // The stand-in for Hugging Face: the pasted link, the live search and a download.
  hf = await startHfStandIn().catch(() => null);
  const cat = await api<{ source: string }>("GET", "/models/catalog");
  viaStandIn = hf !== null && cat.source.startsWith(`http://127.0.0.1:${HF_PORT}`);
  await clean();
});

test.afterAll(async () => {
  if (purposeBefore) {
    const p = purposeBefore;
    // Needs chosen by hand go back as they were; otherwise the sentence alone.
    const manual = p.needs.join() !== p.parsed.join();
    await api("PUT", "/models/purpose", manual ? { text: p.text, needs: p.needs } : { text: p.text });
  }
  await clean();
  // The app server keeps its connection to the stand-in open (keep-alive); close() alone would
  // wait for it until the hook times out.
  hf?.closeAllConnections();
  await new Promise<void>((r) => (hf ? hf.close(() => r()) : r()));
});

/** The tab Modell with both panels open: "Meine Modelle" and "Beste Einstellung finden". */
async function modelTab(page: Page) {
  const tab = page.locator(".mx-simple");
  for (const button of await tab.locator(".mx-actions > button").all()) {
    if ((await button.getAttribute("aria-expanded")) !== "true") await button.click();
  }
  // The list, or the note that the model folders hold none (the suite's model lies outside them).
  await expect(tab.locator('[data-spot="local-models"]').locator(".mp-list, .help").first()).toBeVisible();
  await expect(tab.locator('[data-spot="benchmark"] .bm-head')).toBeVisible();
  await expect(tab.locator('[data-spot="purpose-advice"] .pu-cols')).toBeVisible();
  return tab;
}

/** The tab Experte once profiles, the editor's estimate, the folders, the scan and the engines are there. */
async function expertTab(page: Page) {
  const tab = page.locator(".mx-expert");
  await expect(tab.locator('[data-spot="expert-profile"]')).toBeVisible();
  await expect(tab.locator(".ms-mem").first()).toBeVisible();
  await expect(tab.locator('[data-spot="pc"] .pc-pick')).toBeVisible();
  await expect(tab.locator('[data-spot="engines"] .mp-list')).toBeVisible();
  return tab;
}

/** "Neues Profil" opened under the profiles. */
async function newProfile(page: Page) {
  await page.locator('[data-spot="profile-new"]').click();
  const form = page.locator("form.mp-new");
  await expect(form).toBeVisible();
  return form;
}

/** "Engine herunterladen" opened, with the releases (from GitHub) or what kept them away. */
async function engineDownload(page: Page) {
  await expertTab(page);
  await page.locator('[data-spot="engines"] .btn-row .btn').click();
  const box = page.locator(".pc-dl");
  await expect(box.locator(".pc-variants, .bad, .help").first()).toBeVisible({ timeout: 30_000 });
  return box;
}

/** The tab Vorschläge once the catalog, the picks and the purpose's answer are there. */
async function suggestionsTab(page: Page) {
  const tab = page.locator(".sg");
  await expect(tab.locator(".sg-cards")).toBeVisible({ timeout: 30_000 });
  await expect(tab.locator(".pu-answer")).toBeVisible();
  await expect(tab.locator('[aria-busy="true"]')).toHaveCount(0);
  return tab;
}

/** The phone's filter sheet. */
async function filterSheet(page: Page) {
  await suggestionsTab(page);
  await page.locator(".sug-toggle").click();
  const sheet = page.locator("dialog.sug-sheet");
  await expect(sheet.locator(".sug-filter")).toBeVisible();
  return sheet;
}

/** A pasted Hugging Face link (the stand-in's test repo) with its three sizes. */
async function pastedLink(page: Page) {
  test.skip(!viaStandIn, "needs the Hugging Face stand-in (RP_HF_BASE, see README)");
  await suggestionsTab(page);
  const box = page.locator('[data-spot="hf-link"]');
  await box.locator("input").fill(`https://huggingface.co/${LINK_REPO}`);
  await box.locator('button[type="submit"]').click();
  await expect(box.locator(".tp .tp-go")).toBeVisible();
  return box;
}

/** "Mehr finden" with a result and its files opened. */
async function findMore(page: Page) {
  test.skip(!viaStandIn, "needs the Hugging Face stand-in (RP_HF_BASE, see README)");
  await suggestionsTab(page);
  const box = page.locator('[data-spot="find-more"]');
  await box.locator("input").fill("tiny");
  await box.locator('button[type="submit"]').click();
  await box.locator(".fm-row .fm-top .btn").first().click();
  await expect(box.locator(".fm-open .tp")).toBeVisible();
  return box;
}

/** A download of the stand-in's test repo in the list (running or done). */
async function downloadList(page: Page) {
  test.skip(!viaStandIn, "needs the Hugging Face stand-in (RP_HF_BASE, see README)");
  await clean();
  await api("POST", "/models/downloads", { repo: LINK_REPO, files: ["Tiny-Q6_K.gguf"] });
  const box = page.locator('[data-spot="downloads"]');
  await expect(box.locator(".dl-row")).toBeVisible();
  return box;
}

/** The user's own words: the purpose sentence, file names and paths, the start options typed by hand.
 *  (The catalog's notes on a model's writing and its sources, the server's driver notice and its note
 *  on the model are read too: they come in the app's language.) */
const OWN = ["textarea", ".mf-path", ".mm-file", ".mp-file"];

const screens = (form: Screen["form"]): Screen[] => [
  { name: "Einstellungen › Modell", form, hash: "/settings/model", region: modelTab, skip: OWN },
  { name: "Einstellungen › Experte", form, hash: "/settings/expert", region: expertTab, skip: OWN },
  { name: "Einstellungen › Vorschläge", form, hash: "/settings/suggestions", region: suggestionsTab, skip: OWN },
];

leftoverTests("models", [
  ...screens("desktop"),
  { name: "Neues Profil", form: "desktop", hash: "/settings/expert", region: newProfile, skip: OWN },
  { name: "Engine herunterladen", form: "desktop", hash: "/settings/expert", region: engineDownload, skip: OWN },
  { name: "Hugging-Face-Link", form: "desktop", hash: "/settings/suggestions", region: pastedLink, skip: OWN },
  { name: "Mehr finden", form: "desktop", hash: "/settings/suggestions", region: findMore, skip: OWN },
  { name: "Downloads", form: "desktop", hash: "/settings/suggestions", region: downloadList, skip: OWN },
  ...screens("phone"),
  { name: "Filter", form: "phone", hash: "/settings/suggestions", region: filterSheet, skip: OWN },
  { name: "Hugging-Face-Link", form: "phone", hash: "/settings/suggestions", region: pastedLink, skip: OWN },
  { name: "Downloads", form: "phone", hash: "/settings/suggestions", region: downloadList, skip: OWN },
  ...screens("tablet"),
]);

// ---------------------------------------------------------------------------------------------
// Review screenshots in English and Russian at 1280 and 375 px; no tab may scroll sideways.

const SIZES = [
  { width: 1280, height: 800 },
  { width: 375, height: 812 },
];
const TABS: [string, string, (page: Page) => Promise<unknown>][] = [
  ["model", "/settings/model", modelTab],
  ["expert", "/settings/expert", expertTab],
  ["suggestions", "/settings/suggestions", suggestionsTab],
];

for (const lang of ["en", "ru"] as const) {
  test.describe(`models layout (${lang})`, () => {
    test.beforeAll(() => api("PUT", "/settings", { language: lang }));
    test.afterAll(() => api("PUT", "/settings", { language: "de" }));

    for (const size of SIZES) {
      test(`Modell, Experte, Vorschläge bei ${size.width} px: nichts ragt seitlich heraus`, async ({ page, open }, info) => {
        test.skip(info.project.name !== "desktop-graphit", "one run is enough: the window size is set here");
        test.setTimeout(120_000);
        await page.setViewportSize(size);
        for (const [name, hash, ready] of TABS) {
          await open(hash);
          await expect(page.locator("html")).toHaveAttribute("lang", lang);
          await ready(page);
          // Neither the page nor the tab's own scroller is wider than the window.
          const wide = await page.evaluate(() => {
            const out: string[] = [];
            const doc = document.documentElement;
            if (doc.scrollWidth > doc.clientWidth + 1) out.push(`page ${doc.scrollWidth} > ${doc.clientWidth}`);
            for (const el of document.querySelectorAll<HTMLElement>(".view, .set-panel, .set-row")) {
              const style = getComputedStyle(el);
              if (style.overflowX === "visible" && el.scrollWidth > el.clientWidth + 1) out.push(`${el.className} ${el.scrollWidth} > ${el.clientWidth}`);
            }
            return out;
          });
          expect(wide, `${name} at ${size.width} px (${lang})`).toEqual([]);
          if (SHOTS) {
            const dir = SHOTS;
            mkdirSync(dir, { recursive: true });
            await page.screenshot({ path: join(dir, `${name}-${lang}-${size.width}.png`), fullPage: false });
            // Further down: the benchmark (Modell), the PC and engines (Experte), the catalog (Vorschläge).
            const below = name === "model" ? '[data-spot="benchmark"]' : name === "expert" ? '[data-spot="pc"]' : ".sug-panel";
            await page.locator(below).first().scrollIntoViewIfNeeded();
            await page.screenshot({ path: join(dir, `${name}-${lang}-${size.width}-b.png`), fullPage: false });
          }
        }
      });
    }
  });
}
