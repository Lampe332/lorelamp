// How a chat is set and coloured (Q3): the arrangement "Messenger" (their messages left, yours
// right, both as bubbles), the inks of spoken words and of actions (app-wide in the settings tab
// "Chat" with its live sample, and per chat in "Aussehen"), and the character's large picture, which
// must never show as a slice of itself: checked at 870, 1024 and 1280 px window width, on a phone
// and on a tablet.
//
// Needs `RP_E2E_DATA` (the chats are filled through the test database, so no model has to answer).
// Every test works in its own chat, deletes it afterwards and puts the app-wide look back.
// `RP_E2E_SHOTS` (optional): review screenshots go there.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { api, charId, deleteChat, newChat } from "./api";
import { dataDir, fillChat } from "./db";
import { expect, isPhone, test } from "./fixtures";

const ILSE = "Kapitän Ilse Brandt";
const shotsDir = process.env.RP_E2E_SHOTS ?? "";

let chats: number[] = [];
test.beforeEach(() => {
  test.skip(!dataDir, "needs RP_E2E_DATA (the test server's data folder)");
});
test.afterEach(async () => {
  for (const id of chats) await deleteChat(id);
  chats = [];
  await api("PUT", "/chat-look", {});
});

/** A new chat with `total` messages: the greeting, then yours and the character's in turn. */
async function filledChat(name: string, total: number): Promise<number> {
  const id = await newChat(await charId(name));
  chats.push(id);
  fillChat(id, total);
  return id;
}

async function shot(page: Page, name: string, project: string) {
  if (!shotsDir) return;
  mkdirSync(shotsDir, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.mouse.move(0, 0);
  await page.screenshot({ path: join(shotsDir, `${name}-${project}.png`) });
}

const look = () => api<{ look: Record<string, unknown> }>("GET", "/chat-look").then((r) => r.look);
const chatLook = (id: number) => api<{ look: Record<string, unknown> }>("GET", `/chats/${id}/look`).then((r) => r.look);
const root = (page: Page) => page.locator(".chat-main.chat");
const tab = (page: Page) => page.getByLabel("Darstellung aller Chats");
const sample = (page: Page) => page.getByRole("region", { name: "Vorschau" });
const panel = (page: Page) => page.getByRole("dialog", { name: "Aussehen dieses Chats" });
async function openLook(page: Page) {
  if (isPhone(page)) await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.getByRole("button", { name: "Aussehen dieses Chats" }).click();
  await expect(panel(page)).toBeVisible();
}
const color = (at: Locator) => at.evaluate((el) => getComputedStyle(el).color);
/** What a CSS colour comes to at this place of the page (theme tokens and the character's hue resolved). */
const resolved = (at: Locator, css: string) =>
  at.evaluate((el, value) => {
    const probe = document.createElement("span");
    probe.style.color = value;
    el.appendChild(probe);
    const c = getComputedStyle(probe).color;
    probe.remove();
    return c;
  }, css);
/** What makes the page scroll sideways: nothing, or the elements that reach past the window. */
const sideScroll = (page: Page) =>
  page.evaluate(() => {
    if (document.documentElement.scrollWidth <= window.innerWidth) return [];
    const out = [`page ${document.documentElement.scrollWidth} > ${window.innerWidth}`];
    for (const el of document.querySelectorAll<HTMLElement>("body *")) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.right > window.innerWidth + 0.5) out.push(`${el.tagName.toLowerCase()}.${String(el.className).trim().split(" ").join(".")} ${Math.round(r.left)}-${Math.round(r.right)}`);
    }
    return out.slice(0, 12);
  });
/** The page stands still (a phone slides a new screen in) and nothing reaches past the window. */
const settled = (page: Page) => expect.poll(() => sideScroll(page)).toEqual([]);

/** Left and right edge of every message's text, with whose it is, and the column they stand in. */
const edges = (page: Page, scope: string) =>
  page.evaluate((sel) => {
    const col = document.querySelector(`${sel} .col`)!.getBoundingClientRect();
    const msgs = [...document.querySelectorAll(`${sel} .msg`)].map((m) => {
      const p = m.querySelector(".prose")!.getBoundingClientRect();
      const av = m.querySelector(".msg-head .av")?.getBoundingClientRect();
      return { mine: m.classList.contains("user"), left: p.left, right: p.right, bottom: p.bottom, bg: getComputedStyle(m.querySelector(".prose")!).backgroundColor, av: av ? { left: av.left, right: av.right, bottom: av.bottom } : null };
    });
    return { col: { left: col.left, right: col.right, width: col.width }, msgs };
  }, scope);

test("Anordnung Messenger: ihre Nachrichten links, meine rechts, beide als Blasen; je Chat änderbar", async ({ page, open }, info) => {
  const id = await filledChat("Mara Voss", 5);
  await open("/settings/chat");
  const styles = tab(page).getByRole("radiogroup", { name: "Anordnung" });
  await expect(styles.getByRole("radio")).toHaveCount(5);
  await expect(styles.getByRole("radio", { name: "Buch" })).toHaveAttribute("aria-checked", "true");
  await styles.getByRole("radio", { name: "Messenger", exact: true }).click();
  await expect(styles.getByRole("radio", { name: "Messenger", exact: true })).toHaveAttribute("aria-checked", "true");
  await expect.poll(look).toEqual({ style: "sides" });

  // The sample beside the controls follows at once.
  await expect(sample(page).locator("[data-cv-style]")).toHaveAttribute("data-cv-style", "sides");
  const inSample = await edges(page, ".cv-sample-box");
  expect(inSample.msgs.map((m) => m.mine)).toEqual([false, true, false]);
  expect(inSample.msgs[1].right).toBeGreaterThan(inSample.col.right - 2);
  expect(inSample.msgs[0].left).toBeLessThan(inSample.col.left + 60);
  await shot(page, "einstellungen-messenger", info.project.name);

  // Arrow keys move through the arrangements like through any group of radio buttons.
  await styles.getByRole("radio", { name: "Messenger", exact: true }).focus();
  await page.keyboard.press("ArrowRight");
  await expect.poll(look).toEqual({ style: "wide" });
  await page.keyboard.press("ArrowLeft");
  await expect.poll(look).toEqual({ style: "sides" });

  await open(`/chat/${id}`);
  await expect(root(page)).toHaveAttribute("data-cv-style", "sides");
  await expect(page.getByRole("article")).toHaveCount(5);
  await settled(page);
  const e = await edges(page, ".stream");
  const mid = (e.col.left + e.col.right) / 2;
  for (const m of e.msgs) {
    // A bubble of its own colour, never the bare page.
    expect(m.bg).not.toBe("rgba(0, 0, 0, 0)");
    expect(m.right - m.left).toBeLessThanOrEqual(e.col.width * 0.92);
    if (m.mine) {
      expect(m.right, "mine: at the right edge").toBeGreaterThan(e.col.right - 50);
      expect(m.left, "mine: not across the whole column").toBeGreaterThan(e.col.left + 20);
      if (m.av) expect(m.av.left, "my picture right of my bubble").toBeGreaterThanOrEqual(m.right);
    } else {
      expect(m.left, "theirs: at the left edge").toBeLessThan(e.col.left + 50);
      expect(m.right, "theirs: not across the whole column").toBeLessThan(e.col.right - 20);
      expect(m.av, "their picture").not.toBeNull();
      expect(m.av!.right, "their picture left of their bubble").toBeLessThanOrEqual(m.left);
      expect(Math.abs(m.av!.bottom - m.bottom), "the picture stands at the bubble's foot").toBeLessThan(3);
    }
  }
  // Theirs start left of the middle, mine end right of it.
  expect(e.msgs.filter((m) => !m.mine).every((m) => m.left < mid)).toBe(true);
  expect(e.msgs.filter((m) => m.mine).every((m) => m.right > mid)).toBe(true);
  await settled(page);
  await shot(page, "chat-messenger", info.project.name);

  // This chat alone reads as a book again; the app keeps "Messenger".
  await openLook(page);
  await panel(page).getByRole("radiogroup", { name: "Anordnung" }).getByRole("radio", { name: "Buch" }).click();
  await expect(root(page)).toHaveAttribute("data-cv-style", "book");
  await expect.poll(() => chatLook(id)).toEqual({ style: "book" });
  expect(await look()).toEqual({ style: "sides" });
  await panel(page).getByRole("button", { name: "Anordnung: Wie überall" }).click();
  await expect(root(page)).toHaveAttribute("data-cv-style", "sides");
  await expect.poll(() => chatLook(id)).toEqual({});
});

test("Farben: Rede und Handlung bekommen eigene Farben, in der Vorschau sofort, im Chat und je Chat", async ({ page, open }, info) => {
  const id = await newChat(await charId(ILSE)); // her greeting holds "speech" and *actions*
  chats.push(id);
  await open("/settings/chat");
  const box = sample(page).locator(".cv-sample-box");
  const say = box.locator(".msg.ai .cv-say").first();
  const act = box.locator(".msg.ai em").first();
  const mySay = box.locator(".msg.user .cv-say").first();
  const myAct = box.locator(".msg.user em").first();
  const sayInks = tab(page).getByRole("radiogroup", { name: "Farbe der Rede" });
  const actInks = tab(page).getByRole("radiogroup", { name: "Farbe der Handlung" });

  // As before: speech in the character's colour, actions in the quiet ink, yours untouched.
  const figure = await resolved(box, "hsl(var(--h, 30) var(--hue-name))");
  const story = await resolved(box, "var(--story)");
  const quiet = await resolved(box, "var(--narration)");
  await expect(sayInks.getByRole("radio", { name: "Farbe der Figur" })).toHaveAttribute("aria-checked", "true");
  await expect(actInks.getByRole("radio", { name: "Zurückhaltend" })).toHaveAttribute("aria-checked", "true");
  expect(await color(say)).toBe(figure);
  expect(await color(act)).toBe(quiet);
  expect(await color(mySay)).toBe(story);

  // Speech in a fixed blue, actions in the character's colour.
  await sayInks.getByRole("radio", { name: "Blau" }).click();
  const blue = await resolved(box, "hsl(215 var(--hue-name))");
  expect(blue).not.toBe(figure);
  await expect.poll(() => color(say)).toBe(blue);
  await actInks.getByRole("radio", { name: "Farbe der Figur" }).click();
  await expect.poll(() => color(act)).toBe(figure);
  await expect.poll(look).toEqual({ say: "h215", act: "figure" });
  expect(await color(mySay), "my own messages stay as they are").toBe(story);
  expect(await color(myAct)).toBe(quiet);

  // Any other hue from the slider; upright actions; heavier speech; my own messages too.
  await tab(page).getByRole("slider", { name: "Eigener Farbton" }).first().fill("300");
  await expect.poll(look).toEqual({ say: "h300", act: "figure" });
  const pink = await resolved(box, "hsl(300 var(--hue-name))");
  await expect.poll(() => color(say)).toBe(pink);
  await expect(sayInks.getByRole("radio", { checked: true })).toHaveCount(0);
  expect(await act.evaluate((el) => getComputedStyle(el).fontStyle)).toBe("italic");
  await tab(page).getByRole("switch", { name: /Handlung kursiv/ }).click();
  await expect.poll(() => act.evaluate((el) => getComputedStyle(el).fontStyle)).toBe("normal");
  await tab(page).getByRole("switch", { name: /Rede kräftiger/ }).click();
  await expect.poll(() => say.evaluate((el) => getComputedStyle(el).fontWeight)).toBe("600");
  await tab(page).getByRole("switch", { name: /Auch in meinen Nachrichten/ }).click();
  await expect.poll(() => color(mySay)).toBe(pink);
  await expect.poll(() => color(myAct)).toBe(figure);
  await expect.poll(look).toEqual({ say: "h300", act: "figure", act_italic: false, say_bold: true, mine: true });
  await shot(page, "einstellungen-farben", info.project.name);

  // The chat reads the same way.
  await open(`/chat/${id}`);
  const first = page.getByRole("article", { name: "Nachricht der Figur" }).first();
  const chatSay = first.locator(".cv-say").first();
  const chatAct = first.locator("em").first();
  const here = root(page);
  await expect.poll(() => color(chatSay)).toBe(await resolved(here, "hsl(300 var(--hue-name))"));
  expect(await color(chatAct)).toBe(await resolved(here, "hsl(var(--h, 30) var(--hue-name))"));
  expect(await chatAct.evaluate((el) => getComputedStyle(el).fontStyle)).toBe("normal");
  expect(await chatSay.evaluate((el) => getComputedStyle(el).fontWeight)).toBe("600");

  // One chat with its own inks: red speech, actions in the text's ink.
  await openLook(page);
  await panel(page).getByRole("radiogroup", { name: "Farbe der Rede" }).getByRole("radio", { name: "Rot" }).click();
  await panel(page).getByRole("radiogroup", { name: "Farbe der Handlung" }).getByRole("radio", { name: "Textfarbe" }).click();
  await expect.poll(() => chatLook(id)).toEqual({ say: "h5", act: "text" });
  await expect.poll(() => color(chatSay)).toBe(await resolved(here, "hsl(5 var(--hue-name))"));
  await expect.poll(() => color(chatAct)).toBe(await resolved(here, "var(--story)"));
  if (!isPhone(page)) await shot(page, "chat-farben", info.project.name);

  // Speech not set apart at all: its ink controls go, the words read like the rest.
  await panel(page).getByRole("switch", { name: /Rede hervorheben/ }).click();
  await expect(panel(page).getByRole("radiogroup", { name: "Farbe der Rede" })).toHaveCount(0);
  await expect.poll(() => color(chatSay)).toBe(await resolved(here, "var(--story)"));
  await expect(panel(page).getByRole("radiogroup", { name: "Farbe der Handlung" })).toBeVisible();
});

test("Einstellungen › Chat: Gruppen, Absatzabstand, Bilder an den Nachrichten, über Strg K zu finden", async ({ page, open }, info) => {
  await open("/settings/chat");
  for (const group of ["Schrift", "Nachrichten", "Farben", "Antworten"]) await expect(tab(page).getByRole("region", { name: group })).toBeVisible();
  const box = sample(page).locator(".cv-sample-box");
  const gap = () => box.locator(".msg.ai .prose p").first().evaluate((p) => parseFloat(getComputedStyle(p).marginBottom) / parseFloat(getComputedStyle(p).fontSize));
  expect(Math.round((await gap()) * 100) / 100).toBe(0.85);
  await tab(page).getByRole("slider", { name: "Absatzabstand" }).fill("1.5");
  await expect.poll(async () => Math.round((await gap()) * 100) / 100).toBe(1.5);
  await expect(box.locator(".msg.ai .msg-head .av").first()).toBeVisible();
  await tab(page).getByRole("switch", { name: /Bilder an den Nachrichten/ }).click();
  await expect(box.locator(".msg.ai .msg-head .av").first()).toBeHidden();
  await expect.poll(look).toEqual({ para: 1.5, avatars: false });
  await settled(page);
  await shot(page, "einstellungen-chat-gruppen", info.project.name);

  // What the server does not know is refused, nothing is stored.
  await expect(api("PUT", "/chat-look", { say: "#ff0000" })).rejects.toThrow(/400.*bad_look/);
  await expect(api("PUT", "/chat-look", { act: "h360" })).rejects.toThrow(/400.*bad_look/);
  await expect(api("PUT", "/chat-look", { style: "left" })).rejects.toThrow(/400.*bad_look/);
  expect(await look()).toEqual({ para: 1.5, avatars: false });

  if (!isPhone(page)) {
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox").fill("farbe rede");
    await page.getByRole("option", { name: /Farbe der Rede/ }).first().click();
    await expect(page).toHaveURL(/#\/settings\/chat$/);
    await expect(tab(page).getByRole("radiogroup", { name: "Farbe der Rede" })).toBeInViewport();
    await page.keyboard.press("Control+k");
    await page.getByRole("combobox").fill("meine nachrichten rechts");
    await page.getByRole("option", { name: /Meine Nachrichten rechts, ihre links/ }).first().click();
    await expect(tab(page).getByRole("radiogroup", { name: "Anordnung" })).toBeInViewport();
  }
  await page.getByRole("button", { name: "Alles auf Standard" }).click();
  await expect.poll(look).toEqual({});
});

/** The large picture: where it stands, and how much of the picture it shows (1 = all of it). */
const stageView = (page: Page) =>
  page.getByRole("complementary", { name: `Bild von ${ILSE}` }).evaluate((el) => {
    const img = el.querySelector("img") as HTMLImageElement;
    const r = el.getBoundingClientRect();
    const pic = img.naturalWidth / img.naturalHeight;
    const area = r.width / r.height;
    const share = getComputedStyle(img).objectFit === "contain" ? 1 : Math.min(pic / area, area / pic);
    const story = document.querySelector(".stream")!.getBoundingClientRect();
    return { share, left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height, vw: innerWidth, vh: innerHeight, story: { left: story.left, right: story.right, top: story.top, width: story.width } };
  });

async function stageOn(page: Page) {
  const toggle = page.getByRole("button", { name: `Bild von ${ILSE} groß zeigen` });
  if ((await toggle.getAttribute("aria-pressed")) !== "true") await toggle.click();
  const pic = page.getByRole("complementary", { name: `Bild von ${ILSE}` }).locator("img");
  await expect.poll(() => pic.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth > 0)).toBe(true);
}

test("Großes Bild der Figur: ganz im Fenster und nie nur ein Ausschnitt (870, 1024, 1280 px und Handy)", async ({ page, open }, info) => {
  const id = await filledChat(ILSE, 3);
  const sizes = isPhone(page) ? [page.viewportSize()!] : [870, 1024, 1280].map((width) => ({ width, height: 800 }));
  for (const size of sizes) {
    await page.setViewportSize(size);
    await open(`/chat/${id}`);
    await stageOn(page);
    // The picture's place follows the chat's own width (the sidebar takes its share of the window).
    const side = (await page.locator(".cv-frame").evaluate((el) => el.clientWidth)) >= 600;
    await expect(page.locator(".cv-frame")).toHaveAttribute("data-cv-stage", side ? "side" : "band");
    await expect.poll(async () => (await stageView(page)).share, `${size.width}: at least 70 % of the picture`).toBeGreaterThanOrEqual(0.7);
    const v = await stageView(page);
    expect(v.left, `${size.width}: inside the window`).toBeGreaterThanOrEqual(0);
    expect(v.right).toBeLessThanOrEqual(v.vw + 0.5);
    expect(v.top).toBeGreaterThanOrEqual(0);
    expect(v.bottom).toBeLessThanOrEqual(v.vh + 0.5);
    if (side) {
      expect(v.left, "beside the story").toBeGreaterThanOrEqual(v.story.right - 1);
      expect(v.width).toBeGreaterThanOrEqual(240);
    } else {
      expect(v.bottom, "a band above the story").toBeLessThanOrEqual(v.story.top + 1);
      expect(v.height).toBeGreaterThanOrEqual(120);
    }
    expect(v.story.width, "the story keeps room to read").toBeGreaterThanOrEqual(320);
    await expect(page.getByRole("textbox", { name: `Nachricht an ${ILSE}` })).toBeInViewport();
    await settled(page);
    await shot(page, `bild-gross-${size.width}`, info.project.name);
  }
  await page.getByRole("complementary", { name: `Bild von ${ILSE}` }).getByRole("button", { name: "Großes Bild ausblenden" }).click();
  await expect(page.getByRole("complementary", { name: `Bild von ${ILSE}` })).toHaveCount(0);
});

test.describe("Tablet 1024x1366", () => {
  test.use({ viewport: { width: 1024, height: 1366 }, hasTouch: true, isMobile: true });

  test("Messenger und Farben am Tablet: beide Seiten, große Tippflächen, nichts läuft seitlich", async ({ page, open }, info) => {
    test.skip(info.project.name.startsWith("phone-"), "sets its own window size; one run per theme is enough");
    const id = await filledChat(ILSE, 5);
    await api("PUT", "/chat-look", { style: "sides", say: "h215" });
    await open(`/chat/${id}`);
    await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
    await expect(root(page)).toHaveAttribute("data-cv-style", "sides");
    await settled(page);
    const e = await edges(page, ".stream");
    expect(e.msgs.filter((m) => m.mine).every((m) => m.right > e.col.right - 60)).toBe(true);
    expect(e.msgs.filter((m) => !m.mine).every((m) => m.left < e.col.left + 60)).toBe(true);
    await settled(page);
    // Every action under the last reply is a full finger's width, the two new ones included.
    const last = page.getByRole("article").last();
    for (const name of ["Neu generieren", "Weiter schreiben", "Text kopieren", "Nachricht bearbeiten", "Bis hier zurückspulen", "Ab hier abzweigen"]) {
      const b = (await last.getByRole("button", { name }).boundingBox())!;
      expect(Math.min(b.width, b.height), name).toBeGreaterThanOrEqual(48);
    }
    await shot(page, "tablet-messenger", info.project.name);
    // "Aussehen" on a tablet: the arrangement tiles and the ink swatches are finger-sized too.
    await page.getByRole("button", { name: "Aussehen dieses Chats" }).click();
    const blue = panel(page).getByRole("radiogroup", { name: "Farbe der Rede" }).getByRole("radio", { name: "Blau" });
    await blue.scrollIntoViewIfNeeded();
    await expect(blue).toHaveAttribute("aria-checked", "true");
    const sw = (await blue.boundingBox())!;
    expect(Math.min(sw.width, sw.height)).toBeGreaterThanOrEqual(48);
    await shot(page, "tablet-aussehen", info.project.name);
  });
});
