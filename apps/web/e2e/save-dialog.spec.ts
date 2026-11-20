// Exports through the save dialog (`showSaveFilePicker`, the desktop window and desktop browsers): the
// dialog opens first, while the click is fresh, then the file is fetched and written to the chosen
// place. Card export as PNG / JSON V2 / JSON V3, several cards as one ZIP, a persona as PNG, the chat's
// own version of a card; a cancelled dialog saves nothing and shows no error; without the API (a phone
// over Wi-Fi) the plain download is used.

import { readFile } from "node:fs/promises";
import type { Page } from "@playwright/test";
import { baseURL, charId, deleteChat, library, newChat, token } from "./api";
import { expect, test } from "./fixtures";
import { recordSavePicker, saves, setSaveAnswer, withoutSavePicker, type Saved } from "./save-picker";

/** A file straight from the API, to compare with what the page saved. */
async function fromApi(path: string, init: RequestInit = {}): Promise<Buffer> {
  const res = await fetch(`${baseURL}/api${path}`, { ...init, headers: { Authorization: `Bearer ${token}`, ...init.headers } });
  expect(res.ok).toBe(true);
  return Buffer.from(await res.arrayBuffer());
}

const bytes = (s: Saved) => Buffer.from(s.base64, "base64");
const PNG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

/** The names of the files in a ZIP, from its central directory. */
function zipNames(zip: Buffer): string[] {
  let end = zip.length - 22;
  while (end >= 0 && zip.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < 0) throw new Error("not a zip");
  const names: string[] = [];
  let at = zip.readUInt32LE(end + 16);
  for (let i = zip.readUInt16LE(end + 10); i > 0; i--) {
    const nameLen = zip.readUInt16LE(at + 28);
    names.push(zip.subarray(at + 46, at + 46 + nameLen).toString("utf8"));
    at += 46 + nameLen + zip.readUInt16LE(at + 30) + zip.readUInt16LE(at + 32);
  }
  return names;
}

/** Opens the card export of the character page, picks `format` and clicks "Exportieren". */
async function exportCard(page: Page, format: string) {
  await page.getByRole("button", { name: "Exportieren …" }).click();
  const dialog = page.getByRole("dialog", { name: "Figur exportieren" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("radio", { name: new RegExp(`^${format}`) }).check();
  await dialog.getByRole("button", { name: "Exportieren", exact: true }).click();
  return dialog;
}

test.describe("mit Speichern-Dialog", () => {
  test.beforeEach(({ page }) => recordSavePicker(page));

  test("Karte als PNG, JSON V2 und JSON V3", async ({ page, open }) => {
    const ilse = await charId("Kapitän Ilse Brandt");
    await open(`/char/${ilse}/settings`);
    const cases = [
      { format: "PNG", id: "png", ext: "png", mime: "image/png" },
      { format: "JSON V2", id: "json_v2", ext: "json", mime: "application/json" },
      { format: "JSON V3", id: "json_v3", ext: "json", mime: "application/json" },
    ];
    for (const [i, c] of cases.entries()) {
      const dialog = await exportCard(page, c.format);
      await expect(dialog).toHaveCount(0);
      await expect(page.locator(".toast")).toContainText(`Gespeichert: Kapitän Ilse Brandt.${c.ext}`);
      const all = await saves(page);
      expect(all).toHaveLength(i + 1);
      const s = all[i];
      expect(s.suggestedName).toBe(`Kapitän Ilse Brandt.${c.ext}`);
      expect(Object.keys(s.types?.[0]?.accept ?? {})).toEqual([c.mime]);
      expect(s.activated).toBe(true);
      expect(s.fetchedBefore).toBe(i); // the dialog came before this file's request
      expect(s.closed).toBe(true);
      const want = await fromApi(`/characters/${ilse}/export?format=${c.id}`);
      if (c.ext === "png") {
        expect(bytes(s).subarray(0, 8)).toEqual(PNG);
        expect(bytes(s)).toEqual(want);
      } else {
        expect(JSON.parse(bytes(s).toString("utf8"))).toEqual(JSON.parse(want.toString("utf8")));
        expect(JSON.parse(bytes(s).toString("utf8"))).toMatchObject({ spec: c.id === "json_v2" ? "chara_card_v2" : "chara_card_v3", data: { name: "Kapitän Ilse Brandt" } });
      }
    }
  });

  test("die Fassung eines Chats meldet sich neben dem Knopf", async ({ page, open }) => {
    const mara = await charId("Mara Voss");
    const chat = await newChat(mara);
    try {
      await open(`/chat/${chat}/card`);
      await page.getByRole("button", { name: "Diese Fassung exportieren" }).click();
      const dialog = page.getByRole("dialog", { name: "Diese Fassung exportieren" });
      await dialog.getByRole("radio", { name: /^JSON V2/ }).check();
      await dialog.getByRole("button", { name: "Exportieren", exact: true }).click();
      await expect(dialog).toHaveCount(0);
      await expect(page.getByRole("status").filter({ hasText: "Gespeichert: Mara Voss.json" })).toBeVisible();
      const [s] = await saves(page);
      expect(JSON.parse(bytes(s).toString("utf8"))).toMatchObject({ spec: "chara_card_v2", data: { name: "Mara Voss" } });
    } finally {
      await deleteChat(chat);
    }
  });

  test("mehrere Karten als ZIP", async ({ page, open }) => {
    await open("/chars");
    await page.getByRole("button", { name: "Auswählen", exact: true }).click();
    await page.getByRole("checkbox", { name: "Mara Voss auswählen" }).check();
    await page.getByRole("checkbox", { name: "Moos auswählen" }).check();
    const bar = page.getByRole("toolbar", { name: "Auswahl" });
    await bar.getByRole("button", { name: "Exportieren" }).click();
    await bar.getByRole("button", { name: "PNG" }).click();
    await expect.poll(async () => (await saves(page)).length).toBe(1);
    const [s] = await saves(page);
    expect(s.suggestedName).toBe("karten.zip");
    expect(Object.keys(s.types?.[0]?.accept ?? {})).toEqual(["application/zip"]);
    expect(s.activated).toBe(true);
    expect(s.fetchedBefore).toBe(0);
    await expect.poll(async () => (await saves(page))[0].closed).toBe(true);
    const zip = bytes((await saves(page))[0]);
    expect(zip.subarray(0, 2).toString("latin1")).toBe("PK");
    const names = zipNames(zip);
    expect(names).toHaveLength(2);
    expect(names.every((n) => n.endsWith(".png"))).toBe(true);
  });

  test("Persona als PNG", async ({ page, open }) => {
    const second = (await library()).personas.find((p) => p.name === "Testperson Zwei")!;
    await open(`/personas/${second.id}`);
    await page.getByRole("button", { name: "Exportieren (PNG)" }).click();
    await expect(page.locator(".toast")).toContainText("Gespeichert: Testperson Zwei.png");
    const [s] = await saves(page);
    expect(s.suggestedName).toBe("Testperson Zwei.png");
    expect(Object.keys(s.types?.[0]?.accept ?? {})).toEqual(["image/png"]);
    expect(s.activated).toBe(true);
    expect(s.fetchedBefore).toBe(0);
    expect(bytes(s)).toEqual(await fromApi(`/personas/${second.id}/export.png`));
  });

  test("Abbrechen speichert nichts und zeigt keinen Fehler", async ({ page, open }) => {
    const exports: string[] = [];
    page.on("request", (r) => {
      if (/\/export/.test(r.url())) exports.push(r.url());
    });
    const ilse = await charId("Kapitän Ilse Brandt");
    await open(`/char/${ilse}/settings`);
    await setSaveAnswer(page, "cancel");
    const dialog = await exportCard(page, "PNG");
    // The export dialog stays open and usable, without an error.
    await expect(dialog.getByRole("button", { name: "Exportieren", exact: true })).toBeEnabled();
    await expect(dialog.getByRole("alert")).toHaveCount(0);
    await dialog.getByRole("button", { name: "Abbrechen" }).click();
    await expect(dialog).toHaveCount(0);

    const second = (await library()).personas.find((p) => p.name === "Testperson Zwei")!;
    await open(`/personas/${second.id}`);
    await setSaveAnswer(page, "cancel");
    const button = page.getByRole("button", { name: "Exportieren (PNG)" });
    await button.click();
    await expect(button).toBeEnabled();
    await page.waitForTimeout(300);
    await expect(page.locator(".toast")).toHaveCount(0);
    expect(await saves(page)).toEqual([]);
    expect(exports).toEqual([]);
  });
});

test.describe("ohne Speichern-Dialog", () => {
  test.beforeEach(({ page }) => withoutSavePicker(page));

  test("Karte und Persona kommen als Download", async ({ page, open }) => {
    const ilse = await charId("Kapitän Ilse Brandt");
    await open(`/char/${ilse}/settings`);
    expect(await page.evaluate(() => typeof (window as unknown as { showSaveFilePicker?: unknown }).showSaveFilePicker)).toBe("undefined");
    const [card] = await Promise.all([page.waitForEvent("download"), exportCard(page, "PNG")]);
    expect(card.suggestedFilename()).toMatch(/\.png$/);
    const cardBytes = await readFile(await card.path());
    expect(cardBytes.subarray(0, 8)).toEqual(PNG);
    await expect(page.locator(".toast")).toContainText("Gespeichert:");

    const second = (await library()).personas.find((p) => p.name === "Testperson Zwei")!;
    await open(`/personas/${second.id}`);
    const [persona] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "Exportieren (PNG)" }).click()]);
    expect(persona.suggestedFilename()).toMatch(/\.png$/);
    expect(await readFile(await persona.path())).toEqual(await fromApi(`/personas/${second.id}/export.png`));
  });
});

