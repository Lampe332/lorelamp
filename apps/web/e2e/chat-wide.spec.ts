// The wider choice of how a chat looks: the layout "Messenger breit" (theirs at the chat's left
// edge, yours at its right, across the whole width; checked at 1280, 1920 and 2560 px and on a
// 375 px phone), and the finer settings (bubbles: fill, width, corners, see-through, frosted glass,
// shadow; swapped sides, edge and spacing; inks of narration and thoughts; name, time and picture;
// how new messages come and what shows before a reply). Each is set in Settings → Chat (the sample
// follows at once) or in one chat's "Aussehen", stored on the server and still there after a reload.
//
// Needs `RP_E2E_DATA` (the chats are filled through the test database). The test of new messages
// needs the model (it skips itself when none answers). Every test works in its own chat, deletes it
// afterwards and puts the app-wide look back. `RP_E2E_SHOTS` (optional): review screenshots.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { api, charId, deleteChat, newChat } from "./api";
import { addMessages, dataDir } from "./db";
import { expect, isPhone, test } from "./fixtures";
import { STORY } from "./story";

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

/** A new chat with Mara: her greeting, then the short exchange of story.ts (yours first). */
async function storyChat(): Promise<number> {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  addMessages(id, STORY);
  return id;
}

async function shot(page: Page, name: string, project: string) {
  if (!shotsDir) return;
  mkdirSync(shotsDir, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.mouse.move(0, 0);
  await page.waitForTimeout(450); // new messages and the panel have come in
  await page.screenshot({ path: join(shotsDir, `${name}-${project}.png`) });
}

const look = () => api<{ look: Record<string, unknown> }>("GET", "/chat-look").then((r) => r.look);
const chatLook = (id: number) => api<{ look: Record<string, unknown> }>("GET", `/chats/${id}/look`).then((r) => r.look);
const root = (page: Page) => page.locator(".chat-main.chat");
const tab = (page: Page) => page.getByLabel("Darstellung aller Chats");
const sample = (page: Page) => page.locator(".cv-sample-box");
const panel = (page: Page) => page.getByRole("dialog", { name: "Aussehen dieses Chats" });
async function openLook(page: Page) {
  if (isPhone(page)) await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.getByRole("button", { name: "Aussehen dieses Chats" }).click();
  await expect(panel(page)).toBeVisible();
}
const css = (at: Locator, prop: string) => at.evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), prop);
/** What a CSS colour comes to at this place of the page (theme tokens and the character's hue resolved). */
const resolved = (at: Locator, value: string) =>
  at.evaluate((el, v) => {
    const probe = document.createElement("span");
    probe.style.color = v;
    el.appendChild(probe);
    const c = getComputedStyle(probe).color;
    probe.remove();
    return c;
  }, value);
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
const settled = (page: Page) => expect.poll(() => sideScroll(page)).toEqual([]);

/** Where the chat area and every message's bubble and picture stand, and the message field. */
const layout = (page: Page) =>
  page.evaluate(() => {
    const r = (el: Element | null | undefined) => {
      const b = el?.getBoundingClientRect();
      return b ? { left: b.left, right: b.right, width: b.width, top: b.top } : null;
    };
    const area = r(document.querySelector(".chat-main .cv-main"))!;
    const msgs = [...document.querySelectorAll(".chat-main .stream .msg")].map((m) => ({
      mine: m.classList.contains("user"),
      prose: r(m.querySelector(".prose"))!,
      av: r(m.querySelector(".msg-head .av")),
      row: r(m)!,
    }));
    const stream = document.querySelector<HTMLElement>(".chat-main .stream")!;
    return {
      area,
      msgs,
      box: r(document.querySelector(".chat-main .composer .box"))!,
      readSize: parseFloat(getComputedStyle(document.querySelector(".chat-main .prose")!).fontSize),
      scrollbar: stream.offsetWidth - stream.clientWidth,
    };
  });

/** "Messenger breit" at this window: theirs from the chat's left edge, yours to its right edge,
 *  bubbles no wider than reads well, message field on the same edges, nothing scrolls sideways. */
async function checkWide(page: Page, open: (hash: string) => Promise<void>, id: number) {
  await open(`/chat/${id}`);
  await expect(root(page)).toHaveAttribute("data-cv-style", "wide");
  await expect(page.getByRole("article")).toHaveCount(STORY.length + 1);
  await settled(page);
  const l = await layout(page);
  const phone = isPhone(page);
  // The edge: 12 to 44 px, plus the scrollbar's room on the right.
  const reach = (phone ? 14 : 46) + l.scrollbar;
  for (const m of l.msgs) {
    if (m.mine) {
      expect(l.area.right - m.prose.right, "mine: at the chat's right edge").toBeLessThan(reach + (m.av ? m.av.width + 10 : 0));
      if (m.av) expect(m.av.left, "my picture right of my bubble").toBeGreaterThanOrEqual(m.prose.right);
    } else {
      expect(m.av, "their picture").not.toBeNull();
      expect(m.av!.left - l.area.left, "their picture at the chat's left edge").toBeLessThan(phone ? 16 : 46);
      expect(m.prose.left, "their bubble right of the picture").toBeGreaterThanOrEqual(m.av!.right);
    }
    // A bubble never runs across the whole chat: at most 78 % of its row (88 % on a phone) and about 44 em.
    expect(m.prose.width, "bubble width").toBeLessThanOrEqual(m.row.width * (phone ? 0.89 : 0.79) + 1);
    if (!phone) expect(m.prose.width).toBeLessThanOrEqual(l.readSize * 44 + 2);
  }
  // Theirs and yours really stand apart: the two edges are (almost) the whole chat apart.
  const theirs = l.msgs.filter((m) => !m.mine);
  const mine = l.msgs.filter((m) => m.mine);
  const right = Math.max(...mine.map((m) => (m.av ? m.av.right : m.prose.right)));
  const left = Math.min(...theirs.map((m) => m.av!.left));
  // (A phone keeps 12 px at each side, and the test browser a scrollbar a real phone draws over the text.)
  expect((right - left) / l.area.width, "messages use the chat's width").toBeGreaterThan(phone ? 0.88 : 0.9);
  // The message field shares the edges of the messages.
  expect(Math.abs(l.box.left - left), "field and messages: same left edge").toBeLessThan(3);
  expect(Math.abs(l.box.right - right), "field and messages: same right edge").toBeLessThan(3);
}

for (const size of [
  { width: 1280, height: 800 },
  { width: 1920, height: 1080 },
  { width: 2560, height: 1440 },
]) {
  test.describe(`Messenger breit bei ${size.width} px`, () => {
    test.use({ viewport: size });
    test(`ihre Nachrichten am linken Rand, meine am rechten (${size.width} px)`, async ({ page, open }, info) => {
      test.skip(info.project.name.startsWith("phone-"), "a desktop window");
      const id = await storyChat();
      await api("PUT", "/chat-look", { style: "wide" });
      await checkWide(page, open, id);
      await shot(page, `nachher-wide-${size.width}`, info.project.name);
      // For comparison: "Messenger" keeps both in a column in the middle.
      if (size.width === 1920) {
        await api("PUT", "/chat-look", { style: "sides" });
        await expect(root(page)).toHaveAttribute("data-cv-style", "sides");
        const l = await layout(page);
        const theirs = l.msgs.find((m) => !m.mine)!;
        expect(theirs.av!.left - l.area.left, "sides: in the middle").toBeGreaterThan(200);
      }
    });
  });
}

test("Messenger breit auf dem Handy (375 px), im Aussehen des Chats gewählt", async ({ page, open }, info) => {
  test.skip(!isPhone(page), "a phone");
  const id = await storyChat();
  await open(`/chat/${id}`);
  await openLook(page);
  await panel(page).getByRole("radiogroup", { name: "Anordnung" }).getByRole("radio", { name: "Messenger breit" }).click();
  await expect.poll(() => chatLook(id)).toEqual({ style: "wide" });
  expect(await look()).toEqual({});
  await shot(page, "nachher-panel-wide", info.project.name);
  await page.reload();
  await checkWide(page, open, id);
  await shot(page, "nachher-wide-375", info.project.name);
});

test("Blasen: Farbe, Breite, Ecken, Deckkraft, Milchglas und Schatten, in der Vorschau und im Chat, nach Neuladen erhalten", async ({ page, open }, info) => {
  test.skip(isPhone(page), "the settings tab's controls are the same on a phone; the phone test covers the panel");
  const id = await storyChat();
  await open("/settings/chat");
  const t = tab(page);
  await t.getByRole("radiogroup", { name: "Anordnung" }).getByRole("radio", { name: "Messenger", exact: true }).click();
  await t.getByRole("radiogroup", { name: "Blase der Figur" }).getByRole("radio", { name: "Blau" }).click();
  await t.getByRole("radiogroup", { name: "Deine Blase" }).getByRole("radio", { name: "Keine" }).click();
  await t.getByLabel("Größte Blasenbreite", { exact: true }).fill("60");
  await t.getByLabel("Runde Ecken", { exact: true }).fill("6");
  await t.getByRole("switch", { name: /^Spitze Ecke zur Person/ }).click();
  await t.getByLabel("Deckkraft der Blasen", { exact: true }).fill("70");
  await t.getByLabel("Milchglas", { exact: true }).fill("8");
  await t.getByRole("switch", { name: /^Schatten unter Blasen/ }).click();
  const want = { style: "sides", ai_fill: "h215", me_fill: "none", bubble: 60, corner: 6, tail: false, fill: 70, frost: 8, shadow: true };
  await expect.poll(look).toEqual(want);

  // The sample follows: their bubble blue and see-through, square-ish, with shadow and glass; yours bare.
  const theirs = sample(page).locator(".msg.ai .prose").first();
  const mine = sample(page).locator(".msg.user .prose");
  const bg = await css(theirs, "background-color");
  expect(bg, "70 % cover").toContain("0.7");
  const [r, , b] = bg.match(/[\d.]+/g)!.map(Number);
  expect(b, "a blue").toBeGreaterThan(r);
  expect(await css(mine, "background-color")).toBe("rgba(0, 0, 0, 0)");
  expect(await css(theirs, "border-bottom-left-radius")).toBe("6px");
  expect(await css(theirs, "border-top-right-radius")).toBe("6px");
  expect(await css(theirs, "box-shadow")).not.toBe("none");
  expect(await css(mine, "box-shadow")).toBe("none");
  expect(await css(theirs, "backdrop-filter")).toBe("blur(8px)");
  await shot(page, "nachher-einstellungen-blasen", info.project.name);

  // After a reload the controls show the stored values.
  await page.reload();
  await expect(t.getByRole("radiogroup", { name: "Blase der Figur" }).getByRole("radio", { name: "Blau" })).toHaveAttribute("aria-checked", "true");
  await expect(t.getByLabel("Größte Blasenbreite", { exact: true })).toHaveValue("60");
  await expect(t.getByRole("switch", { name: /^Spitze Ecke zur Person/ })).not.toBeChecked();

  // The chat draws the same; the bubble keeps to 60 % of its row.
  await open(`/chat/${id}`);
  await expect(root(page)).toHaveAttribute("data-cv-shadow", "on");
  const bubble = page.locator(".stream .msg.ai .prose").last();
  expect(await css(bubble, "background-color")).toContain("0.7");
  const [w, row] = await bubble.evaluate((el) => [el.getBoundingClientRect().width, el.closest(".msg")!.getBoundingClientRect().width]);
  expect(w).toBeLessThanOrEqual(row * 0.6 + 1);
  await shot(page, "nachher-chat-blasen", info.project.name);

  // "Standard" gives one field back; "Alles auf Standard" all of them.
  await open("/settings/chat");
  await t.getByRole("button", { name: "Milchglas: Standard" }).click();
  const { frost: _frost, ...rest } = want;
  await expect.poll(look).toEqual(rest);
  await t.getByRole("button", { name: "Alles auf Standard" }).click();
  await expect.poll(look).toEqual({});
});

test("Je Chat: Messenger breit mit getauschten Seiten, Randabstand und Abstand zwischen Nachrichten", async ({ page, open }, info) => {
  test.skip(isPhone(page), "a desktop window");
  const id = await storyChat();
  await api("PUT", "/chat-look", { gap: 40 });
  await open(`/chat/${id}`);
  await openLook(page);
  const p = panel(page);
  // The groups are one click away.
  for (const g of ["Nachrichten", "Schrift", "Farben", "Blasen", "Name, Zeit und Bild", "Antworten"]) await expect(p.getByRole("navigation").getByRole("button", { name: g, exact: true })).toBeVisible();
  await p.getByRole("navigation").getByRole("button", { name: "Antworten", exact: true }).click();
  await expect(p.getByRole("region", { name: "Antworten" })).toBeInViewport();
  await p.getByRole("navigation").getByRole("button", { name: "Nachrichten", exact: true }).click();

  // "Seiten tauschen" and "Abstand zum Rand" only show where they act.
  await expect(p.getByRole("switch", { name: /^Seiten tauschen/ })).toHaveCount(0);
  await p.getByRole("radiogroup", { name: "Anordnung" }).getByRole("radio", { name: "Messenger breit" }).click();
  await p.getByRole("switch", { name: /^Seiten tauschen/ }).click();
  await p.getByLabel("Abstand zum Rand", { exact: true }).fill("80");
  await p.getByLabel("Abstand zwischen Nachrichten", { exact: true }).fill("10");
  await expect.poll(() => chatLook(id)).toEqual({ style: "wide", flip: true, edge: 80, gap: 10 });
  expect(await look()).toEqual({ gap: 40 });
  await p.getByRole("button", { name: "Aussehen schließen" }).click();

  const check = async () => {
    const l = await layout(page);
    for (const m of l.msgs) {
      if (m.mine) expect(m.prose.left - l.area.left, "mine: left now").toBeLessThan(80 + 32 + 10 + 4);
      else expect(l.area.right - m.prose.right, "theirs: right now").toBeLessThan(80 + 32 + 10 + 24);
    }
    expect(await css(page.locator(".stream .msg").first(), "margin-bottom")).toBe("10px");
  };
  await check();
  await shot(page, "nachher-wide-getauscht-1920", info.project.name);
  await page.reload();
  await expect(root(page)).toHaveAttribute("data-cv-flip", "on");
  await check();
});

test("Farben der Erzählung und der Gedanken (`…`)", async ({ page, open }) => {
  test.skip(isPhone(page), "a desktop window");
  const id = await storyChat();
  await open("/settings/chat");
  const t = tab(page);
  await t.getByRole("radiogroup", { name: "Farbe der Erzählung" }).getByRole("radio", { name: "Zurückhaltend" }).click();
  await t.getByRole("radiogroup", { name: "Farbe der Gedanken" }).getByRole("radio", { name: "Gold" }).click();
  await expect.poll(look).toEqual({ story: "soft", think: "h50" });
  await open(`/chat/${id}`);
  // The thought stands without its backticks, in its own ink and italic.
  const thought = page.locator(".stream .msg.ai .cv-think").first();
  await expect(thought).toHaveText("Hoffentlich bleibt er auf einen Kaffee.");
  await expect(page.locator(".stream .msg.ai .prose").nth(1)).not.toContainText("`");
  expect(await css(thought, "color")).toBe(await resolved(thought, "hsl(50 var(--hue-name))"));
  expect(await css(thought, "font-style")).toBe("italic");
  const prose = page.locator(".stream .msg.ai .prose").nth(1);
  expect(await css(prose, "color")).toBe(await resolved(prose, "var(--narration)"));
});

test("Name, Uhrzeit und Bild: Namen aus, Uhrzeit an, Bild groß und eckig", async ({ page, open }, info) => {
  test.skip(isPhone(page), "a desktop window");
  const id = await storyChat();
  await open("/settings/chat");
  const t = tab(page);
  await t.getByRole("switch", { name: /^Namen zeigen/ }).click();
  await t.getByRole("switch", { name: /^Uhrzeit zeigen/ }).click();
  await t.getByLabel("Bildgröße", { exact: true }).fill("48");
  await t.getByRole("radiogroup", { name: "Bildform" }).getByRole("radio", { name: "Eckig" }).click();
  await expect.poll(look).toEqual({ names: false, times: true, av_size: 48, av_shape: "square" });
  await expect(sample(page).locator(".msg-time").first()).toHaveText(/^\d{2}:\d{2}$/);

  await open(`/chat/${id}`);
  const head = page.locator(".stream .msg.ai").last().locator(".msg-head");
  await expect(head.locator(".msg-name")).toBeHidden();
  await expect(head.locator(".msg-time")).toHaveText(/^\d{2}:\d{2}$/);
  const av = head.locator(".av");
  expect(await av.evaluate((el) => [el.getBoundingClientRect().width, getComputedStyle(el).borderTopLeftRadius])).toEqual([48, "4px"]);
  await shot(page, "nachher-chat-zeit-bild", info.project.name);

  // Without pictures, size and shape are not offered.
  await open("/settings/chat");
  await t.getByRole("switch", { name: /^Bilder an den Nachrichten/ }).click();
  await expect(t.getByLabel("Bildgröße", { exact: true })).toHaveCount(0);
  await expect(t.getByRole("radiogroup", { name: "Bildform" })).toHaveCount(0);
});

test("Neue Nachrichten steigen auf, vor der Antwort steht „Mara Voss schreibt …“", async ({ page, open }) => {
  test.skip(isPhone(page), "a desktop window");
  const id = await storyChat();
  await open("/settings/chat");
  const t = tab(page);
  await t.getByRole("radiogroup", { name: "Neue Nachrichten" }).getByRole("radio", { name: "Aufsteigen" }).click();
  await t.getByRole("radiogroup", { name: "Bevor die Antwort kommt" }).getByRole("radio", { name: "Punkte und Name" }).click();
  await expect.poll(look).toEqual({ enter: "rise", typing: "name" });

  await open(`/chat/${id}`);
  await expect(page.getByRole("article")).toHaveCount(STORY.length + 1);
  // What was there already never moves.
  await expect(page.locator(".stream .msg.cv-new")).toHaveCount(0);
  const field = page.locator(".chat-main .composer textarea");
  await expect(field).toBeEnabled();
  await field.fill("„Gern. Wo sind die Schürzen?“");
  // A fast model replaces the typing line within milliseconds, so watch the page for it
  // from before sending instead of looking at it afterwards.
  await page.evaluate(() => {
    const w = window as unknown as { seenTyping: string[] };
    w.seenTyping = [];
    const look = () => {
      for (const e of document.querySelectorAll<HTMLElement>(".stream .msg.ai.cv-new .cv-typing-name")) {
        if (e.checkVisibility() && e.textContent) w.seenTyping.push(e.textContent);
      }
    };
    new MutationObserver(look).observe(document.body, { childList: true, subtree: true, characterData: true });
  });
  await field.press("Enter");
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { seenTyping: string[] }).seenTyping), { timeout: 20_000 })
    .toContain("Mara Voss schreibt …");
  const mineNew = page.locator(".stream .msg.user.cv-new");
  await expect(mineNew).toHaveCount(1);
  expect(await css(mineNew, "animation-name")).toBe("cv-enter-rise");
  // The answer arrives and replaces the dots.
  await expect(page.locator(".stream .msg.ai.cv-new .prose p").first()).toBeVisible({ timeout: 60_000 });
  await expect(page.locator(".stream .msg.ai.cv-new .cv-typing")).toHaveCount(0);
  const stop = page.getByRole("button", { name: "Antwort stoppen" });
  if (await stop.isVisible()) await stop.click();
});

test("Ungültige Werte lehnt der Server ab; Strg+K findet die neuen Einstellungen", async ({ page, open }) => {
  test.skip(isPhone(page), "the palette is the same on a phone");
  for (const bad of [{ style: "grid" }, { ai_fill: "text" }, { enter: "spin" }, { av_shape: "star" }, { think: "plain" }]) {
    await expect(api("PUT", "/chat-look", bad)).rejects.toThrow();
  }
  await open("/");
  for (const [query, label, spot] of [
    ["uhrzeit", "Uhrzeit an Nachrichten", "chat-times"],
    ["messenger breit", "Messenger breit: Nachrichten an beiden Rändern", "chat-style"],
    ["milchglas", "Durchsichtige Blasen und Milchglas", "chat-fill"],
  ]) {
    await page.keyboard.press("Control+k");
    const box = page.getByRole("combobox").last();
    await box.fill(query);
    await expect(page.getByRole("option").first()).toContainText(label);
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/#\/settings\/chat$/);
    await expect(page.locator(`[data-spot="${spot}"]`)).toBeInViewport();
  }
});
