// Pictures and export: a picture set by dropping a file on the character's picture (scaled down before the
// upload, crop starting at the top of a portrait) and by Ctrl+V / a link on a persona's picture; grids
// load the thumbnails; a card exported as PNG (with `chara` and `ccv3`) and JSON V2/V3, the chat's own
// version of a card; personas as ZIP and as PNG; a card taken over as a persona; the Ctrl+K entries.
// Everything a test adds is deleted again, so the other screens keep their test data.
//
// With RP_E2E_SHOTS set to a folder, the new places are also saved there as screenshots for a look.

import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { inflateRawSync } from "node:zlib";
import type { Download, JSHandle, Page } from "@playwright/test";
import { api, baseURL, charId, deleteChat, library, newChat, token } from "./api";
import { LINK_SITE, png as sitePng, startCardSite } from "./card-site-standin";
import { expect, isPhone, test } from "./fixtures";
import { withoutSavePicker } from "./save-picker";

// The plain download (as on a phone); the save dialog has its own tests in save-dialog.spec.ts.
test.beforeEach(({ page }) => withoutSavePicker(page));

const shots = process.env.RP_E2E_SHOTS;
async function shot(page: Page, name: string) {
  if (!shots) return;
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(400); // let fades and the save bar settle
  await page.screenshot({ path: join(shots, `${name}-${test.info().project.name}.png`) });
}

const run = () => `${Date.now().toString(36)}${Math.floor(Math.random() * 1000)}`;

/** A character of its own, so the picture tests leave the shared test cards untouched. */
async function ownCharacter(): Promise<{ id: number; name: string }> {
  const name = `Bildprobe ${run()}`;
  const card = {
    spec: "chara_card_v2",
    spec_version: "2.0",
    data: { name, description: "Eine Testfigur für Bilder.", personality: "ruhig", scenario: "Ein Atelier.", first_mes: "Hallo.", mes_example: "", tags: ["Test"] },
  };
  const info = await api<{ id: number }>("POST", "/characters/import", new TextEncoder().encode(JSON.stringify(card)));
  return { id: info.id, name };
}

/** A picture drawn in the page (two colour fields, so a crop is visible), as a DataTransfer with one file. */
function picture(page: Page, width: number, height: number, name: string): Promise<JSHandle<DataTransfer>> {
  return page.evaluateHandle(
    async ([w, h, n]) => {
      const c = document.createElement("canvas");
      c.width = w;
      c.height = h;
      const g = c.getContext("2d")!;
      g.fillStyle = "#3a7bd5";
      g.fillRect(0, 0, w, h / 2);
      g.fillStyle = "#e0a030";
      g.fillRect(0, h / 2, w, h / 2);
      const blob = await new Promise<Blob>((r) => c.toBlob((b) => r(b!), "image/png"));
      const dt = new DataTransfer();
      dt.items.add(new File([blob], n, { type: "image/png" }));
      return dt;
    },
    [width, height, name] as const,
  );
}

/** Width and height from a PNG's header. */
const pngSize = (b: Buffer) => ({ width: b.readUInt32BE(16), height: b.readUInt32BE(20) });

/** The text chunks of a PNG: keyword → text (tEXt and uncompressed iTXt). */
function pngText(b: Buffer): Map<string, string> {
  expect(b.subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
  const out = new Map<string, string>();
  for (let at = 8; at < b.length; ) {
    const len = b.readUInt32BE(at);
    const type = b.subarray(at + 4, at + 8).toString("latin1");
    const data = b.subarray(at + 8, at + 8 + len);
    if (type === "tEXt") {
      const z = data.indexOf(0);
      out.set(data.subarray(0, z).toString("latin1"), data.subarray(z + 1).toString("latin1"));
    } else if (type === "iTXt") {
      const z = data.indexOf(0);
      // keyword \0 compression-flag method lang \0 translated \0 text
      let p = z + 3;
      p = data.indexOf(0, p) + 1;
      p = data.indexOf(0, p) + 1;
      out.set(data.subarray(0, z).toString("latin1"), data.subarray(p).toString("utf8"));
    }
    at += 12 + len;
  }
  return out;
}

/** A chunk's JSON, written plain or in base64. */
function base64Json(s: string): unknown {
  try {
    return JSON.parse(s);
  } catch {
    return JSON.parse(Buffer.from(s, "base64").toString("utf8"));
  }
}

/** The files in a ZIP (stored or deflated), read from its central directory. */
function unzip(zip: Buffer): Map<string, Buffer> {
  let end = zip.length - 22;
  while (end >= 0 && zip.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < 0) throw new Error("not a zip");
  const count = zip.readUInt16LE(end + 10);
  let at = zip.readUInt32LE(end + 16);
  const files = new Map<string, Buffer>();
  for (let i = 0; i < count; i++) {
    const method = zip.readUInt16LE(at + 10);
    const size = zip.readUInt32LE(at + 20);
    const nameLen = zip.readUInt16LE(at + 28);
    const extraLen = zip.readUInt16LE(at + 30);
    const commentLen = zip.readUInt16LE(at + 32);
    const local = zip.readUInt32LE(at + 42);
    const name = zip.subarray(at + 46, at + 46 + nameLen).toString("utf8");
    const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    const data = zip.subarray(start, start + size);
    files.set(name, method === 8 ? inflateRawSync(data) : Buffer.from(data));
    at += 46 + nameLen + extraLen + commentLen;
  }
  return files;
}

async function save(page: Page, click: () => Promise<void>): Promise<{ download: Download; bytes: Buffer }> {
  const [download] = await Promise.all([page.waitForEvent("download"), click()]);
  return { download, bytes: await readFile(await download.path()) };
}

/** Every export address the page asks for, to check that the token never sits in one. */
function watchExports(page: Page): string[] {
  const urls: string[] = [];
  page.on("request", (r) => {
    if (/\/export/.test(r.url())) urls.push(r.url());
  });
  return urls;
}

/** The stored picture, straight from the API. */
async function storedPicture(path: string): Promise<Buffer> {
  const res = await fetch(`${baseURL}/api${path}`, { headers: { Authorization: `Bearer ${token}` } });
  expect(res.status).toBe(200);
  return Buffer.from(await res.arrayBuffer());
}

test("Bild: Ablegen auf der Figur setzt es, verkleinert und oben ausgeschnitten", async ({ page, open }) => {
  const own = await ownCharacter();
  try {
    await open(`/char/${own.id}/settings`);
    const block = page.locator('[data-spot="char-picture"]');
    await expect(block.getByRole("button", { name: "Bild wählen" })).toBeVisible();
    // A portrait larger than 2048 px: dropped onto the picture.
    const dt = await picture(page, 1200, 3000, "hoch.png");
    await block.locator(".pic-drop").dispatchEvent("dragenter", { dataTransfer: dt });
    await expect(block).toHaveClass(/drop-over/);
    await shot(page, "i6-drop-over");
    await block.locator(".pic-drop").dispatchEvent("drop", { dataTransfer: dt });
    await expect(block).not.toHaveClass(/drop-over/);

    const crop = block.locator(".crop img");
    await expect(crop).toHaveAttribute("src", /^blob:/);
    // Portrait: the crop starts at the top, in the middle.
    await expect(crop).toHaveCSS("object-position", "50% 0%");
    await shot(page, "i6-figur-bild");

    await page.getByRole("button", { name: "Speichern" }).click();
    await expect(page.locator(".savebar").getByRole("status")).toHaveText("Gespeichert");
    const saved = await api<{ info: { custom_avatar: boolean; avatar_pos: [number, number] } }>("GET", `/characters/${own.id}`);
    expect(saved.info.custom_avatar).toBe(true);
    expect(saved.info.avatar_pos).toEqual([50, 0]);
    // Scaled down in the browser before the upload: the long side is 2048 px.
    const size = pngSize(await storedPicture(`/characters/${own.id}/avatar`));
    expect(size).toEqual({ width: 819, height: 2048 });
    // The crop stays movable.
    await block.locator(".crop").focus();
    await page.keyboard.press("ArrowDown");
    await expect(block.locator(".crop img")).toHaveCSS("object-position", "50% 2%");
  } finally {
    await api("DELETE", `/characters/${own.id}`);
  }
});

test("Bild: Strg+V und Bild-Link auf der Persona, mit Hinweis bei gesperrtem Link", async ({ page, open }) => {
  // The server fetches a link's picture itself and refuses every local address, so the picture comes from
  // the stand-in site it may reach (`RP_LINK_TEST_ORIGIN`, see README); never from the internet.
  test.skip(!LINK_SITE, "needs the stand-in site (RP_E2E_LINK_SITE)");
  const site = await startCardSite();
  const persona = await api<{ id: number }>("POST", "/personas", { name: `Bildpersona ${run()}`, pronouns: "", description: "", every_turn: false, card_priority: "persona" });
  try {
    await open(`/personas/${persona.id}`);
    const block = page.locator('[data-spot="persona-picture"]');
    await expect(block.getByRole("button", { name: "Bild wählen" })).toBeVisible();

    // Ctrl+V of a picture while the editor is open (landscape: the crop starts in the middle).
    const dt = await picture(page, 600, 300, "quer.png");
    await page.evaluate((data) => document.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true })), dt);
    const crop = block.locator(".crop img");
    await expect(crop).toHaveAttribute("src", /^blob:/);
    await expect(crop).toHaveCSS("object-position", "50% 50%");
    await block.getByRole("button", { name: "Auswahl verwerfen" }).click();
    await expect(block.locator(".crop")).toHaveCount(0);

    // A link the server may not fetch: a local address (the test server under its other name). The
    // plain advice.
    const other = new URL(baseURL);
    other.hostname = other.hostname === "localhost" ? "127.0.0.1" : "localhost";
    await block.getByRole("button", { name: "Bild-Link einfügen" }).click();
    const field = block.getByRole("textbox", { name: "Adresse des Bilds" });
    await field.fill(`${other.origin}/bild.png`);
    await field.press("Enter");
    await expect(block.getByRole("alert")).toHaveText("Diese Seite gibt das Bild nicht direkt heraus. Bild speichern und hierher ziehen.");
    await shot(page, "i6-persona-link-hinweis");

    // A page that is no picture.
    await field.fill(`${LINK_SITE}/empty`);
    await field.press("Enter");
    await expect(block.getByRole("alert")).toHaveText("Kein Bild (erlaubt: PNG, JPEG, WebP, GIF).");

    // A link that works (a portrait: the crop starts at the top).
    await field.fill(`${LINK_SITE}/img/portrait.png`);
    await field.press("Enter");
    await expect(crop).toHaveAttribute("src", /^blob:/);
    await expect(crop).toHaveCSS("object-position", "50% 0%");
    await expect(block.getByRole("alert")).toHaveCount(0);
    expect(site.seen).toContain("/img/portrait.png");
    await page.getByRole("button", { name: "Speichern" }).click();
    await expect(page.locator(".pe-save").getByRole("status")).toHaveText("Gespeichert");
    const p = (await api<{ personas: { id: number; has_avatar: boolean }[] }>("GET", "/library")).personas.find((x) => x.id === persona.id);
    expect(p?.has_avatar).toBe(true);
    expect(pngSize(await storedPicture(`/personas/${persona.id}/avatar`))).toEqual(pngSize(sitePng()));
  } finally {
    await api("DELETE", `/personas/${persona.id}`);
    await new Promise<void>((resolve) => site.server.close(() => resolve()));
  }
});

test("Raster: Charaktere laden die Vorschaubilder, nicht die Originale", async ({ page, open }) => {
  const ilse = await charId("Kapitän Ilse Brandt");
  const asked: string[] = [];
  page.on("request", (r) => {
    if (/\/api\/characters\/\d+\/(thumb|avatar)/.test(r.url())) asked.push(new URL(r.url()).pathname);
  });
  await open("/chars");
  // The poster cards of the grid (a phone hides the sidebar, which loads them as well).
  await expect(page.locator(`img.art[src*="/characters/${ilse}/thumb"]`)).toBeVisible();
  expect(asked).toContain(`/api/characters/${ilse}/thumb`);
  expect(asked.filter((p) => p.endsWith("/avatar"))).toEqual([]);
  // The thumbnail is a small WebP.
  const thumb = await storedPicture(`/characters/${ilse}/thumb`);
  expect(thumb.subarray(8, 12).toString("latin1")).toBe("WEBP");
  // The character's own page shows the original.
  await open(`/char/${ilse}/chats`);
  await expect(page.locator(`.char-head img[src*="/characters/${ilse}/avatar"]`)).toBeVisible();
});

test("Export: Figur als PNG (chara + ccv3), JSON V2 und JSON V3", async ({ page, open }) => {
  const ilse = await charId("Kapitän Ilse Brandt");
  const urls = watchExports(page);
  await open(`/char/${ilse}/settings`);
  const exportAs = async (format: string) => {
    await page.getByRole("button", { name: "Exportieren …" }).click();
    const dialog = page.getByRole("dialog", { name: "Figur exportieren" });
    await expect(dialog).toBeVisible();
    await dialog.getByRole("radio", { name: new RegExp(`^${format}`) }).check();
    if (format === "PNG") await shot(page, "i6-figur-export");
    const file = await save(page, () => dialog.getByRole("button", { name: "Exportieren", exact: true }).click());
    await expect(dialog).toHaveCount(0);
    return file;
  };

  const png = await exportAs("PNG");
  expect(png.download.suggestedFilename()).toMatch(/\.png$/);
  const chunks = pngText(png.bytes);
  expect(base64Json(chunks.get("chara")!)).toMatchObject({ spec: "chara_card_v2", data: { name: "Kapitän Ilse Brandt" } });
  expect(base64Json(chunks.get("ccv3")!)).toMatchObject({ spec: "chara_card_v3", data: { name: "Kapitän Ilse Brandt" } });

  const v2 = await exportAs("JSON V2");
  expect(v2.download.suggestedFilename()).toMatch(/\.json$/);
  expect(JSON.parse(v2.bytes.toString("utf8"))).toMatchObject({ spec: "chara_card_v2", data: { name: "Kapitän Ilse Brandt" } });

  const v3 = await exportAs("JSON V3");
  expect(JSON.parse(v3.bytes.toString("utf8"))).toMatchObject({ spec: "chara_card_v3", data: { name: "Kapitän Ilse Brandt" } });
  for (const url of urls) expect(url).not.toContain("token=");
});

test("Export: die Fassung der Karte für einen Chat", async ({ page, open }) => {
  const mara = await charId("Mara Voss");
  const chat = await newChat(mara);
  try {
    const { card } = await api<{ card: Record<string, unknown> }>("GET", `/characters/${mara}`);
    await api("PUT", `/chats/${chat}/card`, { ...card, scenario: "Nur in diesem Chat: ein Leuchtturm." });
    await open(`/chat/${chat}/card`);
    await shot(page, "i6-chat-karte");
    await page.getByRole("button", { name: "Diese Fassung exportieren" }).click();
    const dialog = page.getByRole("dialog", { name: "Diese Fassung exportieren" });
    await dialog.getByRole("radio", { name: /^JSON V2/ }).check();
    const file = await save(page, () => dialog.getByRole("button", { name: "Exportieren", exact: true }).click());
    expect(JSON.parse(file.bytes.toString("utf8"))).toMatchObject({ data: { name: "Mara Voss", scenario: "Nur in diesem Chat: ein Leuchtturm." } });
  } finally {
    await deleteChat(chat);
  }
});

test("Export: alle Personas als ZIP und eine als PNG", async ({ page, open }) => {
  const lib = await library();
  const urls = watchExports(page);
  await open("/personas");
  const zip = await save(page, () => page.getByRole("button", { name: "Alle exportieren (ZIP)" }).click());
  expect(zip.download.suggestedFilename()).toMatch(/\.zip$/);
  const files = unzip(zip.bytes);
  const index = [...files.keys()].find((n) => /^personas_.*\.json$/.test(n));
  expect(index).toBeTruthy();
  const backup = JSON.parse(files.get(index!)!.toString("utf8")) as { personas: Record<string, string> };
  expect(Object.values(backup.personas).sort()).toEqual(lib.personas.map((p) => p.name).sort());
  await shot(page, "i6-personas-export");

  const second = lib.personas.find((p) => p.name === "Testperson Zwei")!;
  await open(`/personas/${second.id}`);
  const png = await save(page, () => page.getByRole("button", { name: "Exportieren (PNG)" }).click());
  const persona = pngText(png.bytes).get("persona");
  expect(persona).toBeTruthy();
  expect(JSON.stringify(base64Json(persona!))).toContain("Testperson Zwei");
  for (const url of urls) expect(url).not.toContain("token=");
});

test("Karte als Persona übernehmen, auch über Strg K", async ({ page, open }) => {
  const moos = await charId("Moos");
  const before = new Set((await library()).personas.map((p) => p.id));
  try {
    await open(`/char/${moos}/settings`);
    await page.getByRole("button", { name: "Als Persona übernehmen" }).click();
    await expect(page).toHaveURL(/#\/personas\/\d+$/);
    const added = (await library()).personas.filter((p) => !before.has(p.id));
    expect(added.map((p) => p.name)).toEqual(["Moos"]);
    await expect(page.locator(".pe-title h2")).toContainText("Moos");
    await api("DELETE", `/personas/${added[0].id}`);

    // Ctrl+K on the character's page: the same, and the export dialog.
    await open(`/char/${moos}/settings`);
    const palette = async (text: string, label: string) => {
      await page.keyboard.press("Control+k");
      const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
      await dialog.getByRole("combobox").fill(text);
      await expect(dialog.getByRole("option").first()).toContainText(label);
      await expect(dialog.getByRole("option").first()).not.toContainText(`Persona-${label}`);
      await page.keyboard.press("Enter");
    };
    await palette("figur exportieren", "Figur exportieren");
    await expect(page.getByRole("dialog", { name: "Figur exportieren" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog", { name: "Figur exportieren" })).toHaveCount(0);

    await palette("bild setzen", "Bild setzen");
    await expect(page).toHaveURL(new RegExp(`#/char/${moos}/settings$`));
    await expect(page.locator('[data-spot="char-picture"].spot-on')).toBeVisible();
    await expect(page.getByRole("button", { name: "Bild wählen" })).toBeFocused();

    await palette("als persona übernehmen", "Als Persona übernehmen");
    await expect(page).toHaveURL(/#\/personas\/\d+$/);
    expect((await library()).personas.filter((p) => !before.has(p.id)).map((p) => p.name)).toEqual(["Moos"]);

    await palette("personas exportieren", "Personas exportieren");
    await expect(page).toHaveURL(/#\/personas$/);
    await expect(page.getByRole("button", { name: "Alle exportieren (ZIP)" })).toBeFocused();
    if (isPhone(page)) await shot(page, "i6-personas-liste");
  } finally {
    for (const p of (await library()).personas.filter((p) => !before.has(p.id))) await api("DELETE", `/personas/${p.id}`);
  }
});
