// How a chat looks (Y4): the settings tab "Chat" and a chat's own "Aussehen", the character's
// large picture, backdrop pictures, new text fading in, spoken words, the action mark.
// Every test works in its own new chat, deletes it afterwards and puts app-wide settings back.

import { existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import type { Page } from "@playwright/test";
import { api, charId, deleteChat, newChat } from "./api";
import { dataDir, messageIds } from "./db";
import { expect, isPhone, test } from "./fixtures";

type Frame = { rev: number; x: number; y: number; dim: number; blur: number };
type ChatLook = { character_id: number; global: Record<string, unknown>; look: Record<string, unknown>; backdrop: { mode: string; own: Frame | null; character: Frame | null } };
const chatLook = (id: number) => api<ChatLook>("GET", `/chats/${id}/look`);

const ILSE = "Kapitän Ilse Brandt";
let chats: number[] = [];
test.afterEach(async () => {
  for (const id of chats) {
    await expect(async () => {
      await deleteChat(id);
    }).toPass({ timeout: 120_000, intervals: [1000] });
  }
  chats = [];
  await api("PUT", "/chat-look", {});
  // A character's default backdrop would show in every later chat with her.
  await api("DELETE", `/characters/${await charId(ILSE)}/backdrop`);
});

async function freshChat(name = "Mara Voss"): Promise<number> {
  const id = await newChat(await charId(name));
  chats.push(id);
  return id;
}

/** A picture made here (a dusk sky over dark hills), so no file of anyone's is needed. */
function testPicture(width = 960, height = 540): Buffer {
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
  head.set([8, 2, 0, 0, 0], 8); // 8 bit, RGB
  const rows = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++) {
    const at = y * (width * 3 + 1);
    const t = y / height;
    for (let x = 0; x < width; x++) {
      const hill = 0.62 + 0.09 * Math.sin(x / 70) + 0.05 * Math.sin(x / 23 + 1);
      const sun = Math.max(0, 1 - Math.hypot(x - width * 0.7, y - height * 0.42) / 180);
      const sky = [70 + 150 * t + 80 * sun, 60 + 70 * t + 90 * sun, 110 - 30 * t + 40 * sun];
      const rgb = t > hill ? [22, 30, 34] : sky;
      rows.set(rgb.map((v) => Math.max(0, Math.min(255, Math.round(v)))), at + 1 + x * 3);
    }
  }
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", head), chunk("IDAT", deflateSync(rows)), chunk("IEND", Buffer.alloc(0))]);
}

const shotsDir = process.env.RP_E2E_SHOTS ?? "";
/** A screenshot for the review folder (only when RP_E2E_SHOTS is set). */
async function shot(page: Page, name: string, project: string) {
  if (!shotsDir) return;
  mkdirSync(shotsDir, { recursive: true });
  await page.screenshot({ path: join(shotsDir, `${name}-${project}.png`) });
}

const root = (page: Page) => page.locator(".chat-main.chat");
const prose = (page: Page) => page.getByRole("article", { name: "Nachricht der Figur" }).first().locator(".prose");
const lookButton = (page: Page) => page.getByRole("button", { name: "Aussehen dieses Chats" });
/** Opens "Aussehen". On a phone its button sits in the message field while nothing is typed there. */
async function openLook(page: Page) {
  if (isPhone(page)) await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await lookButton(page).click();
  await expect(panel(page)).toBeVisible();
}
const panel = (page: Page) => page.getByRole("dialog", { name: "Aussehen dieses Chats" });
const sendButton = (page: Page) => page.getByRole("button", { name: "Senden" });
const stopButton = (page: Page) => page.getByRole("button", { name: "Antwort stoppen" });
const fontSize = (page: Page) => prose(page).evaluate((el) => getComputedStyle(el).fontSize);
const fontFamily = (page: Page) => prose(page).evaluate((el) => getComputedStyle(el).fontFamily);

test("Aussehen je Chat: Schrift und Größe bleiben nach Neuladen, andere Chats bleiben wie sie sind", async ({ page, open }) => {
  const id = await freshChat();
  const other = await freshChat();
  await open(`/chat/${id}`);
  await expect(prose(page)).toBeVisible();
  const before = await fontFamily(page);
  expect(before).toContain("Sitka");

  await openLook(page);
  await expect(panel(page)).toBeVisible();
  await panel(page).getByRole("radio", { name: /Georgia/ }).click();
  await panel(page).getByRole("slider", { name: "Schriftgröße" }).fill("23");
  await panel(page).getByRole("slider", { name: "Zeilenabstand" }).fill("1.9");
  await expect.poll(() => fontFamily(page)).toContain("Georgia");
  await expect.poll(() => fontSize(page)).toBe("23px");
  // Saved on the server (a moment after the last change), only what this chat changes.
  await expect.poll(async () => (await chatLook(id)).look).toEqual({ font: "georgia", size: 23, line: 1.9 });
  await page.keyboard.press("Escape");
  await expect(panel(page)).toHaveCount(0);

  await open(`/chat/${id}`);
  await expect.poll(() => fontFamily(page)).toContain("Georgia");
  expect(await fontSize(page)).toBe("23px");
  expect(await prose(page).evaluate((el) => Math.round((parseFloat(getComputedStyle(el).lineHeight) / parseFloat(getComputedStyle(el).fontSize)) * 100) / 100)).toBe(1.9);

  // Another chat still reads in the app's face.
  await open(`/chat/${other}`);
  await expect(prose(page)).toBeVisible();
  expect(await fontFamily(page)).toBe(before);

  // "Wie überall" gives one field back to the app's settings.
  await open(`/chat/${id}`);
  await openLook(page);
  await panel(page).getByRole("button", { name: "Schriftgröße: Wie überall" }).click();
  await expect.poll(async () => (await chatLook(id)).look).toEqual({ font: "georgia", line: 1.9 });
  await expect.poll(() => fontSize(page)).not.toBe("23px");
});

test("Einstellungen › Chat: gilt für alle Chats, ist über Strg K zu finden und geht auf Standard zurück", async ({ page, open }, info) => {
  const id = await freshChat();
  await open("/settings/chat");
  const tab = page.getByLabel("Darstellung aller Chats");
  await expect(tab.getByRole("radio", { name: /Sitka/ })).toHaveAttribute("aria-checked", "true");
  // The sample follows each change at once.
  const sample = page.getByRole("region", { name: "Vorschau" }).locator(".prose").first();
  await tab.getByRole("radio", { name: /Verdana/ }).click();
  await expect.poll(() => sample.evaluate((el) => getComputedStyle(el).fontFamily)).toContain("Verdana");
  await tab.getByRole("slider", { name: "Textbreite" }).fill("560");
  await tab.getByRole("radio", { name: "Blasen" }).click();
  await expect(tab.getByRole("radio", { name: "Blasen" })).toHaveAttribute("aria-checked", "true");
  await expect.poll(async () => (await api<{ look: unknown }>("GET", "/chat-look")).look).toEqual({ font: "verdana", width: 560, style: "bubbles" });
  await page.mouse.move(0, 0);
  await shot(page, "einstellungen-chat", info.project.name);

  await open(`/chat/${id}`);
  await expect.poll(() => fontFamily(page)).toContain("Verdana");
  await expect(root(page)).toHaveAttribute("data-cv-style", "bubbles");
  if (!isPhone(page)) expect(await page.locator(".stream .col").evaluate((el) => el.getBoundingClientRect().width)).toBe(560);

  // Ctrl+K finds the setting and opens its place.
  if (!isPhone(page)) {
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox").fill("schriftgröße chat");
    await page.getByRole("option", { name: /Schriftgröße im Chat/ }).first().click();
    await expect(page).toHaveURL(/#\/settings\/chat$/);
    await expect(page.getByRole("slider", { name: "Schriftgröße" })).toBeVisible();
  } else {
    await open("/settings/chat");
  }
  await page.getByRole("button", { name: "Alles auf Standard" }).click();
  await expect.poll(async () => (await api<{ look: unknown }>("GET", "/chat-look")).look).toEqual({});
  await expect(page.getByLabel("Darstellung aller Chats").getByRole("radio", { name: /Sitka/ })).toHaveAttribute("aria-checked", "true");
});

test("Großes Bild der Figur: Klick auf das Bild oben blendet es ein und aus, gemerkt, das Eingabefeld bleibt", async ({ page, open }, info) => {
  const id = await freshChat(ILSE);
  await open(`/chat/${id}`);
  const toggle = page.getByRole("button", { name: "Bild von Kapitän Ilse Brandt groß zeigen" });
  const stage = page.getByRole("complementary", { name: "Bild von Kapitän Ilse Brandt" });
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await expect(stage).toHaveCount(0);

  await toggle.click();
  await expect(stage).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  const pic = stage.locator("img");
  await expect.poll(() => pic.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth > 0)).toBe(true);
  // The original picture, not the small thumbnail.
  expect(await pic.getAttribute("src")).toContain("/avatar?");
  const field = page.getByRole("textbox", { name: "Nachricht an Kapitän Ilse Brandt" });
  await field.fill("Guten Morgen, Ilse.");
  await expect(stage).toBeVisible();
  await expect(field).toBeVisible();
  const s = (await stage.boundingBox())!;
  const story = (await page.locator(".stream").boundingBox())!;
  if (isPhone(page)) {
    expect(s.y + s.height).toBeLessThanOrEqual(story.y + 1); // a band above the story
    expect(s.height).toBeGreaterThanOrEqual(96);
  } else {
    expect(s.x).toBeGreaterThanOrEqual(story.x + story.width - 1); // beside it
    expect(s.width).toBeGreaterThanOrEqual(300);
  }
  await shot(page, "bild-gross", info.project.name);
  // The way out on the picture itself.
  await expect(stage.getByRole("button", { name: "Großes Bild ausblenden" })).toHaveCount(1);

  // Remembered on this device across a reload.
  await page.reload();
  await expect(stage).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await field.fill("");

  // Off again, by keyboard.
  await toggle.focus();
  await page.keyboard.press("Enter");
  await expect(stage).toHaveCount(0);
  await page.reload();
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await expect(stage).toHaveCount(0);
});

test("Hintergrundbild: hochladen, sichtbar, abdunkeln, als Standard der Figur, entfernen", async ({ page, open }, info) => {
  const id = await freshChat(ILSE);
  const character = await charId(ILSE);
  await open(`/chat/${id}`);
  await expect(page.locator(".cv-backdrop")).toHaveCount(0);
  await openLook(page);
  await panel(page).getByLabel("Hintergrundbild hochladen").setInputFiles({ name: "abend.png", mimeType: "image/png", buffer: testPicture() });

  const img = page.locator(".cv-backdrop img");
  await expect(img).toBeVisible();
  await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth)).toBe(960);
  expect(await img.getAttribute("src")).toContain(`/api/chats/${id}/backdrop?`);
  let view = await chatLook(id);
  expect(view.backdrop.mode).toBe("own");
  expect(view.backdrop.own).toMatchObject({ rev: 1, x: 50, y: 50, dim: 60, blur: 0 });
  // The picture is a file in the data folder, stored as JPEG, not a blob in the database.
  if (dataDir) expect(existsSync(join(dataDir, "backdrops", `chat-${id}.jpg`))).toBe(true);
  const served = await page.request.get(await img.evaluate((el: HTMLImageElement) => el.src));
  expect(served.headers()["content-type"]).toBe("image/jpeg");

  // Dim, soften, move: drawn at once, stored a moment later.
  await panel(page).getByRole("slider", { name: "Abdunkeln" }).fill("35");
  await panel(page).getByRole("slider", { name: "Weichzeichnen" }).fill("6");
  await panel(page).getByRole("slider", { name: "Ausschnitt senkrecht" }).fill("20");
  await expect(page.locator(".cv-backdrop i")).toHaveCSS("opacity", "0.35");
  await expect(img).toHaveCSS("filter", "blur(6px)");
  await expect(img).toHaveCSS("object-position", "50% 20%");
  await expect.poll(async () => (await chatLook(id)).backdrop.own).toMatchObject({ dim: 35, blur: 6, y: 20 });
  await shot(page, "hintergrund-aussehen", info.project.name);
  await page.keyboard.press("Escape");
  await shot(page, "hintergrund", info.project.name);

  // After a reload it is still there, with its frame.
  await open(`/chat/${id}`);
  await expect(img).toBeVisible();
  await expect(page.locator(".cv-backdrop i")).toHaveCSS("opacity", "0.35");

  // As the character's default: a new chat with her starts with it.
  await openLook(page);
  await panel(page).getByRole("button", { name: "Als Standard für alle Chats mit Kapitän Ilse Brandt" }).click();
  await expect(panel(page).locator(".cv-status")).toContainText("Neue Chats mit Kapitän Ilse Brandt");
  try {
    const second = await freshChat(ILSE);
    await open(`/chat/${second}`);
    await expect(img).toBeVisible();
    expect(await img.getAttribute("src")).toContain(`/api/characters/${character}/backdrop?`);
    // This chat can do without it.
    await openLook(page);
    await panel(page).getByRole("button", { name: "Keins" }).click();
    await expect(page.locator(".cv-backdrop")).toHaveCount(0);
    expect((await chatLook(second)).backdrop.mode).toBe("none");
  } finally {
    await api("DELETE", `/characters/${character}/backdrop`);
  }

  // Removing the own picture: the chat is plain again and the file is gone.
  await open(`/chat/${id}`);
  await openLook(page);
  await panel(page).getByRole("button", { name: "Entfernen", exact: true }).click();
  await expect(page.locator(".cv-backdrop")).toHaveCount(0);
  view = await chatLook(id);
  expect([view.backdrop.mode, view.backdrop.own, view.backdrop.character]).toEqual(["character", null, null]);
  if (dataDir) expect(existsSync(join(dataDir, "backdrops", `chat-${id}.jpg`))).toBe(false);

  // Not a picture: refused with a plain reason, nothing stored.
  await panel(page).getByLabel("Hintergrundbild hochladen").setInputFiles({ name: "text.png", mimeType: "image/png", buffer: Buffer.from("kein Bild") });
  await expect(panel(page).getByRole("alert")).toContainText("Kein Bild");
  expect((await chatLook(id)).backdrop.own).toBeNull();
});

/** Sends a message and waits until the answer's text is arriving. */
async function startAnswer(page: Page) {
  await page.getByRole("textbox", { name: "Nachricht an Mara Voss" }).fill("Erzähl mir bitte ausführlich und lange von deinem Leuchtturm und dem Meer.");
  await sendButton(page).click();
  await expect(stopButton(page)).toBeVisible();
  const reply = page.getByRole("article", { name: "Nachricht der Figur" }).nth(1);
  await expect.poll(async () => (await reply.locator(".prose").innerText().catch(() => "")).replace("Schreibt …", "").trim().length, { timeout: 60_000, intervals: [50] }).toBeGreaterThan(30);
  return reply;
}

async function stopAnswer(page: Page) {
  if (await stopButton(page).isVisible()) await stopButton(page).click();
  await expect(sendButton(page)).toBeVisible({ timeout: 20_000 });
}

test("Antwort erscheint animiert: neue Textstücke blenden ein, danach bleibt schlichter Text", async ({ page, open }, info) => {
  const id = await freshChat();
  await open(`/chat/${id}`);
  // The greeting was there when the chat opened: never animated.
  await expect(prose(page)).toBeVisible();
  await expect(page.locator(".cv-in")).toHaveCount(0);
  const reply = await startAnswer(page);
  // While the model writes, the newest pieces are fading in (opacity only).
  await expect.poll(() => reply.locator(".cv-in").count(), { timeout: 20_000, intervals: [30] }).toBeGreaterThan(0);
  await expect
    .poll(() => page.evaluate(() => [...document.querySelectorAll(".cv-in")].map((el) => getComputedStyle(el).animationName)[0] ?? null), { timeout: 20_000, intervals: [30] })
    .toBe("cv-in");
  await shot(page, "antwort-animation", info.project.name);
  await stopAnswer(page);
  // Once the last pieces have appeared, the answer is plain text again and nothing of it is hidden.
  await expect(reply.locator(".cv-in")).toHaveCount(0);
  expect((await reply.locator(".prose").innerText()).trim().length).toBeGreaterThan(30);
});

test("Keine Animation, wenn das Gerät Bewegung reduziert oder sie im Chat-Tab aus ist", async ({ page, open }) => {
  const id = await freshChat();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(`/chat/${id}`);
  let reply = await startAnswer(page);
  // Text keeps arriving and no piece of it is ever animated.
  for (let i = 0; i < 12; i++) {
    expect(await page.locator(".cv-in").count()).toBe(0);
    await page.waitForTimeout(60);
  }
  expect((await reply.locator(".prose").innerText()).trim().length).toBeGreaterThan(30);
  await stopAnswer(page);

  // Motion allowed again, but switched off in the settings tab "Chat".
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await open("/settings/chat");
  const off = page.getByRole("switch", { name: /Antwort einblenden/ });
  await expect(off).toBeChecked();
  await off.uncheck();
  await expect.poll(async () => (await api<{ look: unknown }>("GET", "/chat-look")).look).toEqual({ animate: false });
  await open(`/chat/${id}`);
  await page.getByRole("textbox", { name: "Nachricht an Mara Voss" }).fill("Und wie ist das Wetter heute bei dir? Erzähl ausführlich.");
  await sendButton(page).click();
  await expect(stopButton(page)).toBeVisible();
  reply = page.getByRole("article", { name: "Nachricht der Figur" }).last();
  await expect.poll(async () => (await reply.locator(".prose").innerText().catch(() => "")).replace("Schreibt …", "").trim().length, { timeout: 60_000, intervals: [50] }).toBeGreaterThan(20);
  for (let i = 0; i < 12; i++) {
    expect(await page.locator(".cv-in").count()).toBe(0);
    await page.waitForTimeout(60);
  }
  await stopAnswer(page);
});

test("Rede und Handlung: Gesprochenes in der Farbe der Figur, abschaltbar; Sternchen-Knopf im Eingabefeld", async ({ page, open }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA (reads the greeting's id from the test database)");
  const id = await freshChat();
  const [greeting] = messageIds(id);
  await api("PUT", `/messages/${greeting}`, { content: '*Mara sieht auf.* „Du bist früh dran.“ Sie lächelt. "Setz dich", sagt sie, *und schiebt dir einen Becher hin*.' });
  await open(`/chat/${id}`);
  const said = prose(page).locator(".cv-say");
  await expect(said).toHaveText(["„Du bist früh dran.“", '"Setz dich"']);
  await expect(prose(page).locator("em")).toHaveText(["Mara sieht auf.", "und schiebt dir einen Becher hin"]);
  // Nothing of the text is lost or doubled by the markup.
  await expect(prose(page)).toHaveText('Mara sieht auf. „Du bist früh dran.“ Sie lächelt. "Setz dich", sagt sie, und schiebt dir einen Becher hin.');
  const colours = () => prose(page).evaluate((el) => [getComputedStyle(el.querySelector(".cv-say")!).color, getComputedStyle(el).color]);
  let [speech, story] = await colours();
  expect(speech).not.toBe(story);
  // Ctrl+F still marks words inside spoken text.
  await page.keyboard.press("Control+f");
  await page.getByRole("textbox", { name: "Im Chat suchen" }).fill("früh");
  await expect(prose(page).locator("mark")).toHaveText("früh");
  await page.keyboard.press("Escape");

  await openLook(page);
  await panel(page).getByRole("switch", { name: /Rede hervorheben/ }).uncheck();
  await expect(root(page)).toHaveAttribute("data-cv-speech", "off");
  [speech, story] = await colours();
  expect(speech).toBe(story);
  await page.keyboard.press("Escape");

  // The action mark: wraps the selection, or sets a pair with the caret between.
  const field = page.getByRole("textbox", { name: "Nachricht an Mara Voss" });
  await field.fill("nicke");
  await field.selectText();
  await page.getByRole("button", { name: "Als Handlung setzen (Sternchen)" }).click();
  await expect(field).toHaveValue("*nicke*");
  await field.press("End");
  await field.pressSequentially(" Gern.");
  if (!isPhone(page)) {
    await field.press("Control+i");
    await field.pressSequentially("leise");
    await expect(field).toHaveValue("*nicke* Gern.*leise*");
    // Keys typed at once after Ctrl+I land between the marks (the caret was once set a frame late).
    for (let i = 0; i < 8; i++) {
      await field.fill("ja ");
      await field.press("Control+i");
      await page.keyboard.type("gern", { delay: 0 });
      await expect(field).toHaveValue("ja *gern*");
    }
  }
  await field.fill("");
});
