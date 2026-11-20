// Group D "system" in English, Spanish and Russian holds no German (dictionary `system`, see
// docs/i18n.md): the start screen in all five layouts and its layout menu, the import page (files with
// their result, links with a preview and the way by file, SillyTavern with its choice and its result),
// Settings › Sicherungen (with its questions and the choice of several), Settings › Logs, Settings ›
// Handy (phones switched on: QR code and pairing code), and on the Android app's page the connection
// and a part that only runs at the PC. Desktop screens are read at 1280 px, phone screens at 375 px,
// tablet screens at 1280 × 800 with touch; none of them may scroll sideways.
//
// Needs no model answer. Uses, when the test server has them (see e2e/README.md): `RP_E2E_LINK_SITE`
// (the link stand-in, started here), `RP_E2E_ST_ROOT` (a small SillyTavern is built there and removed
// again) and `RP_E2E_LAN=1` (phones switched on and off again); without them those screens read less.
// With `RP_E2E_SHOTS` the English and Russian screens are saved as pictures there.

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import type { Server } from "node:http";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { api, fixture, library, token } from "./api";
import { LINK_SITE, startCardSite } from "./card-site-standin";
import { expect, test } from "./fixtures";
import { leftoverTests, type Screen } from "./leftovers";

const stRoot = process.env.RP_E2E_ST_ROOT;
const LAN = process.env.RP_E2E_LAN === "1";
/** Names of what the tests make end with this mark plus a stamp, so each test makes new ones. */
const MARK = "Q7";
const stamp = () => `${MARK}${Date.now().toString(36)}`;

/** Parts that hold the user's or a card's own words: chat lines, card texts, tags, chat titles. */
const HUB_SKIP = [
  "blockquote",
  ".cl-quote",
  ".l2",
  ".mc-sub",
  ".rc-quote",
  ".fig-blurb",
  ".cmd-fig-blurb",
  ".fig-tags",
  ".hx-line-text > .num", // a figure's short text (or, freshly imported, when)
  ".chat-title",
  ".cl-head > span",
  ".mc-name > span",
  ".rc-who > span",
];

const view = (page: Page) => page.locator("div.view").first();

/** Desktop screens are read at 1280 px. */
async function narrow(page: Page) {
  if ((page.viewportSize()?.width ?? 0) > 1280) await page.setViewportSize({ width: 1280, height: 800 });
}

/** No sideways scrolling, and with RP_E2E_SHOTS (English and Russian) a picture of the screen. */
async function shown(page: Page, name: string, region: Locator): Promise<Locator> {
  await expect(region.first()).toBeVisible();
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth), { message: `${name} scrolls sideways` }).toBeLessThanOrEqual(0);
  const dir = process.env.RP_E2E_SHOTS;
  const lang = await page.locator("html").getAttribute("lang");
  if (dir && lang !== "es") {
    const form = await page.locator("html").getAttribute("data-form");
    const width = page.viewportSize()?.width ?? 0;
    await page.evaluate(() => document.fonts.ready);
    mkdirSync(dir, { recursive: true });
    await page.screenshot({ path: `${dir}/${lang}-${width}-${form === "tablet" ? "tablet-" : ""}${name}.png` });
  }
  return region;
}

// ---------------------------------------------------------------------------------------------
// The start screen

const LAYOUTS = [
  { id: "bisher", root: ".hub.classic" },
  { id: "dashboard", root: ".hub.dash" },
  { id: "bento", root: ".bento-page" },
  { id: "buehne", root: ".stage-page" },
  { id: "kommando", root: ".cmd-page" },
] as const;

/** Picks a layout with the switch on the screen itself (the menu's order is the layouts' order). */
async function layout(page: Page, i: number) {
  await page.locator(".hx-switch").first().click();
  const menu = page.locator(".hx-menu");
  await expect(menu).toBeVisible();
  await menu.locator('[role="menuitemradio"]').nth(i).click();
  await expect(menu).toHaveCount(0);
  await expect(page.locator(LAYOUTS[i].root)).toBeVisible();
}

const startScreens = (form: Screen["form"]): Screen[] => [
  ...LAYOUTS.map((l, i) => ({
    name: `Start: ${l.id}`,
    form,
    hash: "/",
    region: async (page: Page) => {
      if (form === "desktop") await narrow(page);
      await layout(page, i);
      return shown(page, `start-${l.id}`, view(page));
    },
    skip: HUB_SKIP,
  })),
  {
    name: "Start: Layout-Menü",
    form,
    hash: "/",
    region: async (page: Page) => {
      if (form === "desktop") await narrow(page);
      await page.locator(".hx-switch").first().click();
      return shown(page, "start-layout-menu", page.locator(".hx-switch, .hx-menu"));
    },
  },
];

// ---------------------------------------------------------------------------------------------
// Import

const cardFile = (name: string) =>
  JSON.stringify({
    spec: "chara_card_v2",
    spec_version: "2.0",
    data: { name, description: "{{char}} is 41 and repairs old clocks in a small workshop.", personality: "calm, exact", scenario: "", first_mes: "Hello.", mes_example: "", creator: "rp-app-tests", tags: [] },
  });

/** Three files at once: a new card, one that is already in the library, and a note without a card. */
async function importFiles(page: Page) {
  await page.locator('input[type="file"]').first().setInputFiles([
    { name: "selma.json", mimeType: "application/json", buffer: Buffer.from(cardFile(`Selma Rook ${stamp()}`)) },
    { name: "mara_voss.json", mimeType: "application/json", buffer: Buffer.from(fixture("mara_voss.json")) },
    { name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("shopping list") },
  ]);
  await expect(page.locator("#imp-result-title")).toBeVisible();
  await expect(page.locator(".imp-fit, .import-fit").first()).toBeVisible();
  await page.locator(".imp-skip summary").click();
  await expect(page.locator(".imp-skip .imp-row").first()).toBeVisible();
}

/** Links in the field: a page of the stand-in site (a preview) and a site that allows no programs. */
async function pasteLinks(page: Page, links: string[]) {
  const field = page.locator("#imp-link-url");
  await field.fill(links.join("\n"));
  await page.locator(".imp-link-form button[type=submit]").click();
  if (links.length > 1) await expect(page.locator(".lnk-bar .lnk-count span")).toHaveText(/./);
  await expect(page.locator(".imp-wait")).toHaveCount(0);
}
const JANNY = "https://jannyai.com/characters/0b1c2d3e-0000-4000-8000-000000000000_character-a-card";
const BACKYARD = "https://backyard.ai/hub/character/abc123";
const LINK_SKIP = [".imp-pv-desc", ".imp-pv-first", ".imp-pv-tags", "#imp-link-url"];

/** A small SillyTavern: two invented characters (one with a chat), a chat without a card, a
 *  persona; a new one for each test, so nothing in it is in the app yet. */
const installs: string[] = [];
function buildInstall(run: string) {
  const install = join(stRoot!, `SillyTavern-${run}`);
  installs.push(install);
  const user = join(install, "data", "default-user");
  for (const dir of ["characters", "chats/ada", "chats/Niemand", "User Avatars"]) mkdirSync(join(user, dir), { recursive: true });
  writeFileSync(join(user, "characters", "ada.json"), cardFile(`Ada Brenner ${run}`));
  writeFileSync(join(user, "characters", "jorin.json"), cardFile(`Jorin Falk ${run}`));
  const head = JSON.stringify({ user_name: "Jonas", character_name: `Ada Brenner ${run}`, create_date: "2026-05-03@08h00m00s", chat_metadata: {} });
  const line = (is_user: boolean, mes: string, at: string) => JSON.stringify({ name: is_user ? "Jonas" : `Ada Brenner ${run}`, is_user, send_date: at, mes });
  writeFileSync(join(user, "chats/ada", "Ada - 2026-05-03@08h00m00s.jsonl"), [head, line(false, "Hello.", "2026-05-03T06:00:00.000Z"), line(true, "Hi.", "2026-05-03T06:01:00.000Z")].join("\n"));
  writeFileSync(join(user, "chats/Niemand", "Niemand - 2026-05-03@08h00m00s.jsonl"), fixture("sillytavern/chat_orphan.jsonl"));
  writeFileSync(join(user, "User Avatars", "q7.png"), Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64"));
  writeFileSync(join(user, "settings.json"), JSON.stringify({ power_user: { personas: { "q7.png": `Wanderer ${run}` }, persona_descriptions: { "q7.png": { description: "A walker." } } } }));
  return install;
}

async function openStPicker(page: Page) {
  const run = stamp();
  buildInstall(run);
  await page.locator(".imp-st-again").first().click();
  const row = page.locator(".imp-st-row").filter({ hasText: `SillyTavern-${run}` });
  await row.locator("button").click();
  const dialog = page.locator("dialog.x6-dlg");
  await expect(dialog.locator(".x6-grid").first()).toBeVisible();
  return dialog;
}

const importScreens = (form: Screen["form"]): Screen[] => [
  {
    name: "Importieren",
    form,
    hash: "/import",
    region: async (page) => {
      if (form === "desktop") await narrow(page);
      await expect(page.locator(".imp-st .imp-wait")).toHaveCount(0);
      return shown(page, "import", view(page));
    },
  },
];

// ---------------------------------------------------------------------------------------------
// Settings

/** Phones switched on with the switch itself (and off again after the file). */
async function phonesOn(page: Page) {
  const box = page.locator('[data-spot="phone-switch"] input');
  await expect(box).toBeEnabled();
  // The switch follows the server's answer, so it is clicked, not checked.
  if (LAN && !(await box.isChecked())) await box.click();
  if (LAN) await expect(page.locator(".hv-digits")).toBeVisible();
}
/** Adapter names are Windows' own; the address is data. */
const PHONE_SKIP = [".hv-net b", ".hv-url"];

/** The page as the Android app opens it (`&shell=android`): it talks to "another" PC. */
async function asAndroid(page: Page, hash: string) {
  await page.goto(`/?token=${token}&shell=android#${hash}`);
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
}

const LOG_SKIP = [".log-msg", ".log-detail pre"];

const settingsScreens = (form: Screen["form"]): Screen[] => [
  {
    name: "Einstellungen › Sicherungen",
    form,
    hash: "/settings/backups",
    region: async (page) => {
      if (form === "desktop") await narrow(page);
      await expect(page.locator(".bk-row").first()).toBeVisible();
      return shown(page, "backups", view(page));
    },
  },
  {
    name: "Einstellungen › Logs",
    form,
    hash: "/settings/logs",
    region: async (page) => {
      if (form === "desktop") await narrow(page);
      await expect(page.locator(".log-row, .empty-line").first()).toBeVisible();
      return shown(page, "logs", view(page));
    },
    skip: LOG_SKIP,
  },
  {
    name: "Einstellungen › Handy",
    form,
    hash: "/settings/phone",
    region: async (page) => {
      if (form === "desktop") await narrow(page);
      await phonesOn(page);
      return shown(page, "phone", view(page));
    },
    skip: PHONE_SKIP,
  },
];

leftoverTests("system", [
  // ---- desktop
  ...startScreens("desktop"),
  ...importScreens("desktop"),
  {
    name: "Importieren: Ergebnis",
    form: "desktop",
    hash: "/import",
    region: async (page) => {
      await narrow(page);
      await importFiles(page);
      return shown(page, "import-result", view(page));
    },
  },
  {
    name: "Importieren: mehrere Links",
    form: "desktop",
    hash: "/import",
    region: async (page) => {
      await narrow(page);
      await pasteLinks(page, [LINK_SITE ? `${LINK_SITE}/page/${encodeURIComponent(`Ines Quill ${stamp()}`)}` : BACKYARD, JANNY]);
      if (LINK_SITE) await expect(page.locator("article.imp-pv")).toBeVisible();
      await expect(page.locator(".imp-fileonly")).toBeVisible();
      return shown(page, "import-links", view(page));
    },
    skip: LINK_SKIP,
    keep: ["rp-app-tests"],
  },
  {
    name: "SillyTavern: Auswahl",
    form: "desktop",
    hash: "/import",
    region: async (page) => {
      await narrow(page);
      test.skip(!stRoot, "needs RP_ST_SEARCH_ROOTS on the test server and the same folder in RP_E2E_ST_ROOT");
      await openStPicker(page);
      return shown(page, "sillytavern-pick", view(page));
    },
  },
  {
    name: "SillyTavern: Ergebnis",
    form: "desktop",
    hash: "/import",
    region: async (page) => {
      await narrow(page);
      test.skip(!stRoot, "needs RP_ST_SEARCH_ROOTS on the test server and the same folder in RP_E2E_ST_ROOT");
      const dialog = await openStPicker(page);
      await dialog.locator(".x6-foot .btn.primary").click();
      await expect(dialog.locator("#x6-result-h")).toBeVisible();
      return shown(page, "sillytavern-result", view(page));
    },
  },
  ...settingsScreens("desktop"),
  {
    name: "Sicherungen: Wiederherstellen?",
    form: "desktop",
    hash: "/settings/backups",
    region: async (page) => {
      await narrow(page);
      await page.locator(".bk-restore").first().click();
      await expect(page.locator("dialog.bk-dialog[open]")).toBeVisible();
      return shown(page, "backups-restore", page.locator("dialog.bk-dialog[open]"));
    },
  },
  {
    name: "Sicherungen: mehrere löschen?",
    form: "desktop",
    hash: "/settings/backups",
    region: async (page) => {
      await narrow(page);
      await page.locator('[data-spot="backup-pick"] button').first().click();
      await expect(page.locator(".x4-picked")).toBeVisible();
      await shown(page, "backups-pick", view(page));
      await page.locator(".x4-bar-acts .btn.ghost").first().click(); // all
      await page.locator(".x4-bar-acts .btn.danger").click();
      await expect(page.locator("dialog.bk-dialog[open]")).toBeVisible();
      return shown(page, "backups-many", page.locator(".x4-bar, dialog.bk-dialog[open]"));
    },
  },
  {
    name: "Sicherungen: alte löschen?",
    form: "desktop",
    hash: "/settings/backups",
    region: async (page) => {
      await narrow(page);
      await page.locator("#x4-days").fill("x");
      await expect(page.locator("#x4-older-help")).toHaveText(/\d/);
      await page.locator("#x4-days").fill("1");
      await page.locator(".x4-older-go").click();
      await expect(page.locator("dialog.bk-dialog[open], .bk-note").first()).toBeVisible();
      return shown(page, "backups-older", page.locator('[data-spot="backup-room"], dialog.bk-dialog[open]'));
    },
  },
  {
    name: "Logs: Eintrag offen, Leeren fragt",
    form: "desktop",
    hash: "/settings/logs",
    region: async (page) => {
      await narrow(page);
      await page.locator(".log-main").first().click();
      await expect(page.locator(".log-detail")).toBeVisible();
      await page.locator(".log-actions .btn.ghost").click();
      await expect(page.locator(".log-actions .confirm")).toBeVisible();
      return shown(page, "logs-open", view(page));
    },
    skip: LOG_SKIP,
  },

  // ---- phone (375 px)
  ...startScreens("phone"),
  ...importScreens("phone"),
  {
    name: "Importieren: Ergebnis",
    form: "phone",
    hash: "/import",
    region: async (page) => {
      await importFiles(page);
      return shown(page, "import-result", view(page));
    },
  },
  {
    name: "Importieren: Link nur als Datei",
    form: "phone",
    hash: "/import",
    region: async (page) => {
      await pasteLinks(page, [JANNY]);
      await expect(page.locator(".imp-fileonly")).toBeVisible();
      return shown(page, "import-link-file", view(page));
    },
    skip: LINK_SKIP,
  },
  {
    name: "SillyTavern: Auswahl",
    form: "phone",
    hash: "/import",
    region: async (page) => {
      test.skip(!stRoot, "needs RP_ST_SEARCH_ROOTS on the test server and the same folder in RP_E2E_ST_ROOT");
      await openStPicker(page);
      return shown(page, "sillytavern-pick", page.locator("dialog.x6-dlg"));
    },
  },
  ...settingsScreens("phone"),
  {
    name: "Android-App: Verbindung",
    form: "phone",
    hash: "/settings/phone",
    region: async (page) => {
      await asAndroid(page, "/settings/phone");
      await expect(page.locator(".hv-here")).toBeVisible();
      return shown(page, "android-connection", view(page));
    },
  },
  {
    name: "Android-App: Sicherungen nur am PC",
    form: "phone",
    hash: "/settings/backups",
    region: async (page) => {
      await asAndroid(page, "/settings/backups");
      await expect(page.locator(".hv-pc-only")).toBeVisible();
      return shown(page, "android-backups", view(page));
    },
  },

  // ---- tablet (1280 × 800 with touch)
  ...startScreens("tablet"),
  ...importScreens("tablet"),
  ...settingsScreens("tablet"),
]);

// ---------------------------------------------------------------------------------------------
// Setup and clean-up: the start layout and phones go back to how they were; what the tests made
// (a manual backup, imported cards, a persona, the SillyTavern folder) is removed again.

let homeLayout = "bisher";
let backupName = "";
let site: { server: Server } | null = null;
let personasBefore = new Set<number>();
const ours = (name: string) => new RegExp(` ${MARK}[0-9a-z]+$`).test(name);

test.beforeAll(async () => {
  homeLayout = (await library()).home_layout;
  personasBefore = new Set((await library()).personas.map((p) => p.id));
  backupName = (await api<{ name: string }>("POST", "/backups")).name;
  if (LINK_SITE) site = await startCardSite();
});

test.afterEach(async () => {
  await api("PUT", "/settings", { home_layout: homeLayout });
  const lib = await library();
  for (const c of lib.characters.filter((c) => ours(c.name))) await api("DELETE", `/characters/${c.id}`);
  for (const p of lib.personas.filter((p) => !personasBefore.has(p.id))) await api("DELETE", `/personas/${p.id}`);
});

test.afterAll(async () => {
  await api("PUT", "/settings", { home_layout: homeLayout });
  if (LAN) await api("PUT", "/lan", { enabled: false });
  if (backupName) await api("DELETE", `/backups/${encodeURIComponent(backupName)}`).catch(() => {});
  await new Promise<void>((resolve) => (site ? site.server.close(() => resolve()) : resolve()));
  for (const install of installs) rmSync(install, { recursive: true, force: true });
  if (installs.length) await api("GET", "/import/sillytavern/find?refresh=1");
});
