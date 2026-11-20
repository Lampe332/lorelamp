// The card builder (#/chars/new, src/builder/): the way in from the library and Ctrl+K, plan → outline
// → fields → preview → save, the draft (texts and picture) surviving a reload, the new character in
// the library, a chat with it, and its PNG export taken in again unchanged; a copy of a character as
// the start; the model writing a field (needs a model that answers, skips itself otherwise); and the
// builder on a tablet. Cards and pictures here are SFW and made in the test.

import { readFile } from "node:fs/promises";
import { deflateSync } from "node:zlib";
import type { Page } from "@playwright/test";
import { api, library } from "./api";
import { expect, isPhone, test } from "./fixtures";
import { withoutSavePicker } from "./save-picker";

test.beforeEach(({ page }) => withoutSavePicker(page));

type Draft = Record<string, unknown>;
type Info = { id: number; name: string };

let made: number[] = [];
let chats: number[] = [];
test.afterEach(async () => {
  for (const id of chats) await api("DELETE", `/chats/${id}`).catch(() => {});
  for (const id of made) await api("DELETE", `/characters/${id}`).catch(() => {});
  made = [];
  chats = [];
});

/** A portrait PNG drawn here: a dusk gradient (no people, no text). */
function picture(width = 240, height = 320): Buffer {
  const crc = (buf: Buffer) => {
    let c = ~0;
    for (const b of buf) {
      c ^= b;
      for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
    }
    return ~c >>> 0;
  };
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const out = Buffer.alloc(8 + data.length + 4);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(crc(body), 8 + data.length);
    return out;
  };
  const head = Buffer.alloc(13);
  head.writeUInt32BE(width, 0);
  head.writeUInt32BE(height, 4);
  head.set([8, 2, 0, 0, 0], 8);
  const rows = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++) {
    const at = y * (width * 3 + 1);
    for (let x = 0; x < width; x++) {
      const t = y / height;
      rows.set([Math.round(40 + 180 * t), Math.round(60 + 60 * t), Math.round(140 - 60 * t + (x % 7))], at + 1 + x * 3);
    }
  }
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", head), chunk("IDAT", deflateSync(rows)), chunk("IEND", Buffer.alloc(0))]);
}

const draftOf = (id: number) => api<{ draft: Draft }>("GET", `/builder/from/${id}`).then((r) => r.draft);

/** The builder with an empty draft on this device. */
async function fresh(page: Page, open: (hash: string) => Promise<void>) {
  await open("/chars/new");
  await page.evaluate(async () => {
    localStorage.removeItem("rp-builder-draft");
    await new Promise<void>((resolve) => {
      const req = indexedDB.deleteDatabase("rp-builder");
      req.onsuccess = req.onerror = req.onblocked = () => resolve();
    });
  });
  await page.reload();
  await expect(page.getByRole("heading", { level: 1, name: "Neue Figur bauen" })).toBeVisible();
}

const step = (page: Page, name: string) => page.locator(".cb-steps").getByRole("button", { name });
const field = (page: Page, label: string | RegExp) => page.locator(".cb-main").getByLabel(label);

test("Ersteller: Einstieg, Plan, Felder, Vorschau, Speichern, Chat und Export", async ({ page, open }, info) => {
  test.setTimeout(120_000);
  const phone = isPhone(page);
  const name = `Wiebke Sand ${info.project.name}`;

  // The way in: the library's button (on a phone its short label).
  await open("/chars");
  await page.locator(".title-row").getByRole("link", { name: "Neue Figur bauen" }).click();
  await expect(page).toHaveURL(/#\/chars\/new$/);
  if (!phone) {
    // Ctrl+K finds it too.
    await open("/chars");
    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
    await palette.getByRole("combobox").fill("figur bauen");
    await palette.getByRole("option", { name: /Neue Figur bauen/ }).first().click();
    await expect(page).toHaveURL(/#\/chars\/new$/);
  }
  await fresh(page, open);

  // ---- plan → outline
  await field(page, "Idee in einem Satz").fill("Eine Leuchtturmwärterin, die Schiffe zählt und Geschichten sammelt.");
  await page.getByRole("group", { name: "Genre", exact: true }).getByRole("button", { name: "Abenteuer" }).click();
  await expect(page.locator("#cb-genre")).toHaveValue("Abenteuer");
  await page.getByRole("group", { name: "Ton", exact: true }).getByRole("button", { name: "Warm" }).click();
  await field(page, "Beziehung zu dir").fill("{{user}} ist der neue Funker auf der Insel");
  await field(page, "Kernzüge").fill("trocken, geduldig, neugierig");
  await field(page, "Antrieb und Konflikt").fill("Will die Insel nie verlassen, sehnt sich aber nach dem Festland.");
  await expect(page.locator(".cb-outline")).toContainText("Wesen: trocken, geduldig, neugierig");
  await page.getByRole("button", { name: "Gliederung übernehmen" }).click();
  await expect(step(page, /Bauen/)).toHaveAttribute("aria-current", "step");
  await expect(field(page, /^Beschreibung/)).toHaveValue(/\[Aussehen\][\s\S]*\[Wesen\]\ntrocken, geduldig, neugierig/);
  await expect(field(page, /^Persönlichkeit/)).toHaveValue("trocken, geduldig, neugierig");
  await expect(page.locator("#cb-tags")).toHaveValue("Abenteuer, Warm");

  // ---- the fields, the picture
  await page.locator("#cb-name").fill(name);
  await page.locator('[data-spot="builder-picture"] input[type="file"]').setInputFiles({ name: "abend.png", mimeType: "image/png", buffer: picture() });
  await expect(page.locator('[data-spot="builder-picture"] .crop img')).toBeVisible();
  await field(page, /^Szenario/).fill("Ein Herbstabend auf der Insel, das Versorgungsboot ist spät dran.");
  await field(page, /^Erste Nachricht/).fill('*Wiebke stellt zwei Becher Tee auf den Tisch.* "Du bist also {{user}}. Setz dich, der Sturm dauert noch."');
  await page.getByRole("button", { name: "Begrüßung hinzufügen" }).click();
  await page.getByLabel("Begrüßung 1", { exact: true }).fill('"Morgen, {{user}}. Die Möwen waren wieder schneller als du."');
  await field(page, /^Beispieldialoge/).fill("<START>\n{{user}}: Wie viele Schiffe heute?\n{{char}}: Drei. Und ein Wal.");
  await page.getByRole("button", { name: "Eintrag hinzufügen" }).click();
  const lore = page.locator(".cb-lore").first();
  await lore.getByLabel("Titel").fill("Der Leuchtturm");
  await lore.getByLabel("Stichwörter").fill("Leuchtturm, Lampe");
  await lore.getByLabel("Text", { exact: true }).fill("Der Leuchtturm hat 112 Stufen und eine Lampe aus Messing.");
  await page.locator("#cb-tags").fill("Abenteuer, Warm, Insel");
  await page.locator("#cb-tags").blur();

  // ---- the draft survives a reload (texts and picture)
  await page.waitForTimeout(400); // the picture is stored in IndexedDB in the background
  await page.reload();
  await expect(step(page, /Bauen/)).toHaveAttribute("aria-current", "step");
  await expect(page.locator("#cb-name")).toHaveValue(name);
  await expect(field(page, /^Erste Nachricht/)).toHaveValue(/Du bist also \{\{user\}\}/);
  await expect(page.locator('[data-spot="builder-picture"] .crop img')).toBeVisible();
  await step(page, /Planen/).click();
  await expect(field(page, "Idee in einem Satz")).toHaveValue(/Leuchtturmwärterin/);

  // ---- preview: the card as the library shows it, the greeting as the chat shows it
  await step(page, /Vorschau/).click();
  const pv = page.locator(".cb-pv.full");
  await expect(pv.locator(".cc b")).toHaveText(name);
  await expect(pv.locator(".cc img.art")).toBeVisible();
  const lib = await library();
  const me = lib.personas.find((p) => p.id === lib.active_persona_id)?.name ?? "";
  await expect(pv.locator(".cb-pv-msg")).toContainText(`Du bist also ${me}.`);
  await expect(pv.locator(".cb-pv-msg")).not.toContainText("{{user}}");
  await pv.getByRole("button", { name: "Nächste Begrüßung" }).click();
  await expect(pv.locator(".cb-pv-msg")).toContainText("Die Möwen waren wieder schneller");
  await expect(pv.locator(".cb-pv-size")).toContainText("In jedem Prompt");
  await expect(pv.locator(".cb-pv-miss")).toHaveCount(0);

  // ---- save: the new character is in the library (as an import would store it)
  await page.getByRole("button", { name: "Figur speichern" }).click();
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(`${name} ist in deiner Bibliothek`);
  const saved = (await library()).characters.find((c) => c.name === name) as (Info & { has_avatar: boolean }) | undefined;
  expect(saved, "the library lists it").toBeTruthy();
  made.push(saved!.id);
  expect(saved!.has_avatar).toBe(true);
  const stored = await draftOf(saved!.id);
  expect(stored.lore).toEqual([{ name: "Der Leuchtturm", keys: ["Leuchtturm", "Lampe"], content: "Der Leuchtturm hat 112 Stufen und eine Lampe aus Messing.", constant: false, enabled: true }]);
  expect(stored.tags).toEqual(["Abenteuer", "Warm", "Insel"]);
  // Saving cleared the draft on this device.
  expect(await page.evaluate(() => localStorage.getItem("rp-builder-draft"))).toBeNull();

  // ---- export as PNG, taken in again: the same card
  await page.getByRole("button", { name: "Karte exportieren …" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("radio", { name: /^PNG/ }).check();
  const [download] = await Promise.all([page.waitForEvent("download"), dialog.getByRole("button", { name: "Exportieren" }).click()]);
  const png = await readFile(await download.path());
  expect(png.subarray(1, 4).toString("ascii")).toBe("PNG");
  const again = await api<Info>("POST", "/characters/import", new Uint8Array(png));
  made.push(again.id);
  expect(await draftOf(again.id)).toEqual(stored);

  // ---- chat with it at once
  await page.getByRole("button", { name: "Chat starten" }).click();
  await expect(page).toHaveURL(/#\/chat\/\d+$/);
  chats.push(Number(/#\/chat\/(\d+)$/.exec(page.url())![1]));
  await expect(page.locator(".msg.ai").first()).toContainText(`Du bist also ${me}.`);

  // ---- and it stands in the library
  await open("/chars");
  await expect(page.locator(".cc", { hasText: name }).first()).toBeVisible();
});

test("Ersteller: vorhandene Figur als Vorlage", async ({ page, open }) => {
  const source = (await library()).characters.find((c) => c.name === "Mara Voss");
  test.skip(!source, "needs the test character Mara Voss");
  await fresh(page, open);
  await open(`/chars/new/from/${source!.id}`);
  await expect(page).toHaveURL(/#\/chars\/new$/);
  await expect(page.locator("#cb-name")).toHaveValue("Mara Voss (Kopie)");
  const original = await draftOf(source!.id);
  await expect(field(page, /^Beschreibung/)).toHaveValue(String(original.description));
  // With text in the draft, another copy asks first; keeping it changes nothing.
  await open(`/chars/new/from/${source!.id}`);
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.getByRole("button", { name: "Entwurf behalten" }).click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  // Throwing the draft away empties it, on this device too.
  await page.getByRole("button", { name: "Entwurf verwerfen" }).click();
  await page.getByRole("button", { name: "Verwerfen", exact: true }).click();
  await expect(step(page, /Planen/)).toHaveAttribute("aria-current", "step");
  await step(page, /Bauen/).click();
  await expect(page.locator("#cb-name")).toHaveValue("");
});

test("Ersteller: das Modell schreibt ein Feld und schreibt es neu", async ({ page, open }, info) => {
  test.skip(!info.project.name.endsWith("-graphit"), "the model's text does not depend on the theme");
  const engine = await api<{ status: string }>("GET", "/engine");
  test.skip(engine.status !== "ready", "needs a model that answers");
  test.setTimeout(300_000);
  await fresh(page, open);
  await field(page, "Idee in einem Satz").fill("Ein freundlicher Bäcker in einer Hafenstadt, der jeden Morgen Brot an Seeleute verteilt.");
  await page.getByRole("button", { name: "Weiter zum Bauen" }).click();

  // One field: the name, then again; the previous version comes back with one click.
  const name = page.locator("#cb-name");
  const nameField = page.locator('[data-spot="builder-name"]');
  await nameField.getByRole("button", { name: "Schreiben lassen" }).click();
  await expect(nameField.getByRole("status")).toBeVisible();
  await expect(name).not.toHaveValue("", { timeout: 120_000 });
  const first = await name.inputValue();
  expect(first.length).toBeLessThanOrEqual(60);
  await nameField.getByRole("button", { name: "Neu schreiben" }).click();
  await expect(nameField.getByRole("button", { name: "Vorige Fassung" })).toBeVisible({ timeout: 120_000 });
  await nameField.getByRole("button", { name: "Vorige Fassung" }).click();
  await expect(name).toHaveValue(first);

  // A longer field from the plan.
  const scenario = field(page, /^Szenario/);
  await page.locator('[data-spot="cb-scenario"]').getByRole("button", { name: "Schreiben lassen" }).click();
  await expect(scenario).not.toHaveValue("", { timeout: 180_000 });
  expect((await scenario.inputValue()).length).toBeGreaterThan(20);
});

test("Ersteller: Bilder zur Durchsicht", async ({ page, open }, info) => {
  const dir = process.env.RP_E2E_SHOTS;
  test.skip(!dir || !info.project.name.endsWith("-graphit"), "only with RP_E2E_SHOTS, once per window size");
  const phone = isPhone(page);
  if (!phone) await page.setViewportSize({ width: 1280, height: 800 });
  const width = page.viewportSize()!.width;
  const shot = async (name: string) => {
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `${dir}/de-${width}-${name}.png` });
  };
  await fresh(page, open);
  await field(page, "Idee in einem Satz").fill("Eine Leuchtturmwärterin, die Schiffe zählt und Geschichten sammelt.");
  await page.getByRole("group", { name: "Genre", exact: true }).getByRole("button", { name: "Abenteuer" }).click();
  await page.getByRole("group", { name: "Ton", exact: true }).getByRole("button", { name: "Warm" }).click();
  await field(page, "Beziehung zu dir").fill("{{user}} ist der neue Funker auf der Insel");
  await field(page, "Kernzüge").fill("trocken, geduldig, neugierig");
  await shot("builder-plan");
  await page.locator(".cb-outline").scrollIntoViewIfNeeded();
  await shot("builder-outline");
  await page.getByRole("button", { name: "Gliederung übernehmen" }).click();
  await page.locator("#cb-name").fill("Wiebke Sand");
  await page.locator('[data-spot="builder-picture"] input[type="file"]').setInputFiles({ name: "abend.png", mimeType: "image/png", buffer: picture() });
  await field(page, /^Erste Nachricht/).fill('*Wiebke stellt zwei Becher Tee auf den Tisch.* "Du bist also {{user}}. Setz dich, der Sturm dauert noch."');
  await page.locator(".cb-assist").scrollIntoViewIfNeeded();
  await shot("builder-build");
  await page.locator('[data-spot="cb-description"]').scrollIntoViewIfNeeded();
  await shot("builder-fields");
  await step(page, /Vorschau/).click();
  await shot("builder-preview");
  await page.getByRole("button", { name: "Figur speichern" }).click();
  await expect(page.locator(".cb-done")).toBeVisible();
  const id = (await library()).characters.find((c) => c.name === "Wiebke Sand")?.id;
  if (id) made.push(id);
  await shot("builder-saved");
});

test.describe("Ersteller auf dem Tablet", () => {
  test.use({ viewport: { width: 1280, height: 800 }, hasTouch: true, isMobile: true });

  test("Schritte, Felder und Vorschau ohne seitliches Scrollen", async ({ page, open }, info) => {
    test.skip(info.project.name.startsWith("phone-"), "a tablet window");
    await fresh(page, open);
    await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
    const sideways = () => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(await sideways()).toBeLessThanOrEqual(0);
    await field(page, "Idee in einem Satz").fill("Eine Kartografin, die Inseln zeichnet, die es noch nicht gibt.");
    await page.getByRole("button", { name: "Gliederung übernehmen" }).click();
    await page.locator("#cb-name").fill("Tove Lind");
    // Controls are big enough for a finger.
    const box = await page.locator('[data-spot="builder-name"] .cb-write .btn').first().boundingBox();
    expect(box!.height).toBeGreaterThanOrEqual(32);
    expect(await sideways()).toBeLessThanOrEqual(0);
    await step(page, /Vorschau/).click();
    await expect(page.locator(".cb-pv.full .cc b")).toHaveText("Tove Lind");
    expect(await sideways()).toBeLessThanOrEqual(0);
    // No side column on a tablet: the preview is its own step.
    await step(page, /Bauen/).click();
    await expect(page.locator(".cb-side")).toHaveCount(0);
  });
});
