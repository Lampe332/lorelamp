// The import page asks first: a pasted link shows a preview of the card and only "Importieren"
// stores it; SillyTavern installations are found by the server, so nobody has to look for the folder.
//
// Link preview: the test server must never reach Chub or RisuRealm, and its card sources can only be
// moved inside the Rust tests (`Running::link_sources`, see `crates/server/src/link_tests.rs`, which
// cover the preview routes themselves). So the browser's three preview requests are answered here by
// a stand-in with the documented answers; the confirm of the stand-in stores the card through the
// real server, so the library really changes exactly once.
//
// SillyTavern: the test server is started with `RP_ST_SEARCH_ROOTS=<folder>` (never the real drives),
// and the tests get the same folder in `RP_E2E_ST_ROOT`; they build a small SillyTavern there and
// remove it again. Without it they skip themselves.
//
// `RP_E2E_SHOTS` (optional): review screenshots of the preview, the duplicate hint, the find list
// and the empty find go there.

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page, TestInfo } from "@playwright/test";
import { api, charId, fixture, library } from "./api";
import { expect, test } from "./fixtures";

const shots = process.env.RP_E2E_SHOTS;
const stRoot = process.env.RP_E2E_ST_ROOT;

async function shot(page: Page, info: TestInfo, name: string, scrollTo?: string) {
  if (!shots) return;
  if (scrollTo) await page.locator(scrollTo).first().evaluate((el) => el.scrollIntoView({ block: "center" }));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(350); // the preview's short rise
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, `${name}-${info.project.name}.png`) });
}

/** A small SFW card in the V2 format. */
const cardJson = (name: string) =>
  JSON.stringify({
    spec: "chara_card_v2",
    spec_version: "2.0",
    data: {
      name,
      description: `{{char}} hütet den Leuchtturm auf einer kleinen Insel im Norden. Sie kennt jedes Schiff, das vorbeifährt, und schreibt die Namen in ein dickes Logbuch.\n\nAbends kocht sie Tee und erzählt Geschichten vom Meer.`,
      personality: "ruhig, genau, freundlich",
      scenario: "Ein stürmischer Abend im Leuchtturm.",
      first_mes: "*Der Wind rüttelt an der Tür.* Komm rein, bevor der Regen dich mitnimmt. Der Tee ist gleich fertig.",
      mes_example: "",
      creator: "rp-app-tests",
      tags: ["Meer", "Leuchtturm", "Gemütlich", "Abenteuer"],
    },
  });

type StandIn = { previews: number; confirms: number; imported: number[] };

/** Answers the browser's preview requests like the server would for a Chub link. */
async function previewStandIn(page: Page, name: string, duplicateOf: number | null): Promise<StandIn> {
  const seen: StandIn = { previews: 0, confirms: 0, imported: [] };
  const id = `e2e${Date.now().toString(36)}`;
  const picture = Buffer.from(fixture("ilse_brandt.png"));
  await page.route(/\/api\/import\/link\/preview$/, async (route) => {
    seen.previews += 1;
    const { url } = route.request().postDataJSON() as { url: string };
    const card = JSON.parse(cardJson(name)).data;
    await route.fulfill({
      json: {
        status: "preview",
        preview_id: id,
        source: "chub",
        page_url: url,
        card: { name, creator: card.creator, tags: card.tags, description: card.description, first_mes: card.first_mes, creator_notes: "" },
        has_image: true,
        duplicate_of: duplicateOf,
      },
    });
  });
  await page.route(/\/api\/import\/link\/preview\/[^/]+\/image/, (route) => route.fulfill({ status: 200, contentType: "image/png", body: picture }));
  await page.route(/\/api\/import\/link\/confirm$/, async (route) => {
    seen.confirms += 1;
    const { preview_id } = route.request().postDataJSON() as { preview_id: string };
    if (preview_id !== id) return route.fulfill({ status: 404, json: { error: "preview_expired" } });
    const info = await api<{ id: number }>("POST", "/characters/import", new TextEncoder().encode(cardJson(name)));
    seen.imported.push(info.id);
    await route.fulfill({ json: { file: "chub.png", status: "imported", character_id: info.id, source: "chub" } });
  });
  return seen;
}

const LINK = "https://chub.ai/characters/rp-app-tests/leuchtturm-1234abcd";

const pasteOnPage = (page: Page, text: string) =>
  page.evaluate((t) => {
    const data = new DataTransfer();
    data.setData("text/plain", t);
    document.body.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true }));
  }, text);

const pasteInto = (page: Page, selector: string, text: string) =>
  page.locator(selector).evaluate((el, t) => {
    const data = new DataTransfer();
    data.setData("text/plain", t);
    el.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
  }, text);

const byName = async (name: string) => (await library()).characters.filter((c) => c.name === name);

test("Import: ein eingefügter Link zeigt erst die Vorschau, Importieren speichert genau einmal", async ({ page, open }, info) => {
  const name = `Leuchtturmwärterin ${Date.now().toString(36)}`;
  const seen = await previewStandIn(page, name, null);
  const before = (await library()).characters.length;
  try {
    await open("/import");
    await pasteInto(page, "#imp-link-url", LINK);
    const preview = page.getByRole("article", { name });
    await expect(preview).toBeVisible();
    await expect(page.getByRole("textbox", { name: "Link einfügen" })).toHaveValue(LINK);
    await expect(preview.getByText("rp-app-tests")).toBeVisible();
    await expect(preview.getByRole("list", { name: "Tags" }).getByRole("listitem")).toHaveCount(4);
    await expect(preview.getByText(`${name} hütet den Leuchtturm`)).toBeVisible();
    await expect(preview.getByRole("link", { name: "auf Chub" })).toHaveAttribute("href", LINK);
    await expect(preview.locator("img.imp-pv-pic")).toHaveJSProperty("complete", true);
    // The first message is folded away until asked for.
    const first = preview.getByText(/Der Wind rüttelt an der Tür/);
    await expect(first).toBeHidden();
    await preview.getByText("Erste Nachricht").click();
    await expect(first).toBeVisible();
    await expect(preview.getByText("Schon in deiner Bibliothek")).toHaveCount(0);
    await shot(page, info, "vorschau", ".imp-pv");

    // Pasting stored nothing.
    expect(seen.confirms).toBe(0);
    expect((await library()).characters.length).toBe(before);

    await preview.getByRole("button", { name: "Importieren" }).click();
    const done = page.locator(".imp-link-msg").filter({ hasText: "Importiert" });
    await expect(done).toBeVisible();
    await expect(done.getByRole("link", { name })).toHaveAttribute("href", /^#\/char\/\d+\/settings$/);
    await expect(preview).toHaveCount(0);
    await expect(page.getByRole("textbox", { name: "Link einfügen" })).toHaveValue("");
    expect(seen.confirms).toBe(1);
    expect(await byName(name)).toHaveLength(1);
  } finally {
    for (const c of await byName(name)) await api("DELETE", `/characters/${c.id}`);
  }
});

test("Import: Strg+V auf der Seite zeigt die Vorschau, Verwerfen räumt auf", async ({ page, open }) => {
  const name = `Leuchtturmwärterin ${Date.now().toString(36)}`;
  const seen = await previewStandIn(page, name, null);
  await open("/import");
  await pasteOnPage(page, LINK);
  const preview = page.getByRole("article", { name });
  await expect(preview).toBeVisible();
  expect(seen.previews).toBe(1);
  await preview.getByRole("button", { name: "Verwerfen" }).click();
  await expect(preview).toHaveCount(0);
  const field = page.getByRole("textbox", { name: "Link einfügen" });
  await expect(field).toHaveValue("");
  await expect(field).toBeFocused();
  await expect(page.locator(".imp-link-msg")).toHaveCount(0);
  expect(seen.confirms).toBe(0);
  expect(await byName(name)).toHaveLength(0);

  // Typing the link and Enter asks for the preview the same way.
  await field.fill(LINK);
  await field.press("Enter");
  await expect(preview).toBeVisible();
  expect(seen.previews).toBe(2);
  expect(seen.confirms).toBe(0);
});

test("Import: Vorschau einer Karte, die schon in der Bibliothek ist", async ({ page, open }, info) => {
  const mara = await charId("Mara Voss");
  const name = `Leuchtturmwärterin ${Date.now().toString(36)}`;
  const seen = await previewStandIn(page, name, mara);
  try {
    await open("/import");
    await pasteInto(page, "#imp-link-url", LINK);
    const preview = page.getByRole("article", { name });
    const hint = preview.locator(".imp-pv-dup");
    await expect(hint).toContainText("Schon in deiner Bibliothek");
    await expect(hint.getByRole("link", { name: "Mara Voss" })).toHaveAttribute("href", `#/char/${mara}/settings`);
    // Importing anyway stays possible.
    await expect(preview.getByRole("button", { name: "Importieren" })).toBeEnabled();
    await shot(page, info, "vorschau-doppelt", ".imp-pv");
    expect(seen.confirms).toBe(0);
  } finally {
    for (const c of await byName(name)) await api("DELETE", `/characters/${c.id}`);
  }
});

// ---------------------------------------------------------------------------------------------
// SillyTavern found by the server

/** A plain 1×1 PNG without card data (a persona picture). */
const PIXEL = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64");

const refresh = () => api("GET", "/import/sillytavern/find?refresh=1");

test("Import: SillyTavern wird gefunden und übernommen", async ({ page, open }, info) => {
  test.skip(!stRoot, "needs RP_ST_SEARCH_ROOTS on the test server and the same folder in RP_E2E_ST_ROOT");
  const run = Date.now().toString(36);
  const install = join(stRoot!, `SillyTavern-${run}`);
  const user = join(install, "data", "default-user");
  for (const dir of ["characters", "chats/Leuchtturm", "User Avatars"]) mkdirSync(join(user, dir), { recursive: true });
  // The test cards of the fixtures (already in the library: unticked as duplicates) and two new ones.
  writeFileSync(join(user, "characters", "mara_voss.json"), fixture("mara_voss.json"));
  writeFileSync(join(user, "characters", "moos.json"), fixture("moos.json"));
  for (const n of [1, 2]) writeFileSync(join(user, "characters", `st-${n}.json`), cardJson(`ST-Fund ${run} ${n}`));
  writeFileSync(join(user, "chats/Leuchtturm", "chat.jsonl"), "{}");
  writeFileSync(join(user, "User Avatars", "st-persona.png"), PIXEL);
  writeFileSync(
    join(user, "settings.json"),
    JSON.stringify({ power_user: { personas: { "st-persona.png": `Reisende ${run}` }, persona_descriptions: { "st-persona.png": { description: "Eine Reisende aus dem Test." } } } }),
  );
  const personasBefore = new Set((await library()).personas.map((p) => p.id));
  try {
    await refresh();
    await open("/import");
    const block = page.getByRole("region", { name: "Aus SillyTavern übernehmen" });
    const list = block.getByRole("list", { name: "SillyTavern gefunden" });
    const row = list.getByRole("listitem").filter({ hasText: `SillyTavern-${run}` });
    await expect(row).toBeVisible();
    await expect(row).toContainText(`in ${stRoot}`);
    await expect(row).toContainText("Nutzer default-user · 4 Karten · 1 Persona");
    await expect(block.getByRole("button", { name: "Ordner selbst wählen" })).toBeVisible();
    await shot(page, info, "st-gefunden", ".imp-st");

    // "Übernehmen" asks first: the cards already in the library are unticked, the new ones and the persona ticked.
    await row.getByRole("button", { name: "Übernehmen" }).click();
    const dialog = page.getByRole("dialog", { name: "Aus SillyTavern übernehmen" });
    await expect(dialog.getByRole("checkbox", { name: `ST-Fund ${run} 1 übernehmen` })).toBeChecked();
    await expect(dialog.getByRole("checkbox", { name: `ST-Fund ${run} 2 übernehmen` })).toBeChecked();
    await expect(dialog.getByRole("checkbox", { name: "Mara Voss übernehmen" })).not.toBeChecked();
    await expect(dialog.getByRole("checkbox", { name: "Moos übernehmen" })).not.toBeChecked();
    await expect(dialog.getByRole("checkbox", { name: `Reisende ${run} übernehmen` })).toBeChecked();
    // The broken chat without a card is shown, but cannot be chosen.
    await expect(dialog.getByRole("group", { name: "Ohne passende Karte" })).toContainText("chat");
    await expect(dialog.getByRole("group", { name: "Ohne passende Karte" }).getByRole("checkbox")).toHaveCount(0);
    await expect(dialog.locator(".x6-total")).toHaveText("2 Karten · 1 Persona gewählt");
    await dialog.getByRole("button", { name: "Importieren" }).click();
    const sum = (label: string) => dialog.locator(".x6-sum > div").filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
    await expect(sum("Übernommen")).toHaveText("3");
    await expect(sum("Übersprungen")).toHaveText("0");
    await expect(sum("Fehlgeschlagen")).toHaveText("0");
    await dialog.getByRole("button", { name: "Fertig" }).click();
    await expect(dialog).toHaveCount(0);

    const result = page.getByRole("region", { name: "Ergebnis" });
    await expect(result).toBeVisible();
    const figure = (label: string) => result.locator(".imp-sum > div").filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
    await expect(figure("Importiert")).toHaveText("3");
    await expect(figure("Schon vorhanden")).toHaveText("0");
    await expect(figure("Fehler")).toHaveText("0");
    const rows = result.getByRole("list", { name: "Dateien" }).getByRole("listitem");
    await expect(rows.getByRole("link", { name: `ST-Fund ${run} 1`, exact: true })).toHaveAttribute("href", /^#\/char\/\d+\/settings$/);
    await expect(rows.getByRole("link", { name: `Reisende ${run}` })).toHaveAttribute("href", /^#\/personas\/\d+$/);
    expect((await library()).characters.filter((c) => c.name.startsWith(`ST-Fund ${run} `))).toHaveLength(2);
  } finally {
    rmSync(install, { recursive: true, force: true });
    for (const c of (await library()).characters.filter((c) => c.name.startsWith(`ST-Fund ${run} `))) await api("DELETE", `/characters/${c.id}`);
    for (const p of (await library()).personas.filter((p) => !personasBefore.has(p.id))) await api("DELETE", `/personas/${p.id}`);
    await refresh();
  }
});

test("Import: kein SillyTavern gefunden, der Ordner bleibt wählbar", async ({ page, open }, info) => {
  test.skip(!stRoot, "needs RP_ST_SEARCH_ROOTS on the test server and the same folder in RP_E2E_ST_ROOT");
  mkdirSync(stRoot!, { recursive: true });
  await refresh();
  await open("/import");
  const block = page.getByRole("region", { name: "Aus SillyTavern übernehmen" });
  await expect(block).toContainText("Auf dem PC wurde kein SillyTavern gefunden.");
  await expect(block.getByRole("list", { name: "SillyTavern gefunden" })).toHaveCount(0);
  await expect(block.getByRole("button", { name: "Ordner selbst wählen" })).toBeVisible();
  await shot(page, info, "st-nichts", ".imp-st");

  // A search again shows the calm line while it runs, then the same answer.
  await block.getByRole("button", { name: "Erneut suchen" }).click();
  await expect(block).toContainText("Auf dem PC wurde kein SillyTavern gefunden.");
  const chooser = page.waitForEvent("filechooser");
  await block.getByRole("button", { name: "Ordner selbst wählen" }).click();
  await chooser;
});
