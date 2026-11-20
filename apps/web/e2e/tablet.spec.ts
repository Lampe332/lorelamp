// The tablet layout (src/tablet/, contract in src/app/formfactor.ts): a window 768–1366 px wide used
// with a finger, or the Android app (`shell=android`), gets a navigation rail, list and detail side
// by side and a chat with the figure in its own place. Checked at four sizes (two tablets, each
// upright and sideways) with touch: navigation, list + detail (characters, settings), the chat with
// its picture, turning the tablet without losing anything, the on-screen keyboard, touch sizes, and
// swiping through the variants of a reply.
//
// The tests bring their own window size, so they run once per theme (the desktop projects) and skip
// themselves in the phone projects. `RP_E2E_SHOTS` (optional): review screenshots of every rebuilt
// screen go there. The swipe test needs `RP_E2E_ST_ROOT` like swipes.spec.ts.

import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { Page } from "@playwright/test";
import { api, charId, deleteChat, setLayout, token } from "./api";
import { expect, test } from "./fixtures";

const shots = process.env.RP_E2E_SHOTS;
const stRoot = process.env.RP_E2E_ST_ROOT;

const SIZES = [
  { width: 800, height: 1280 },
  { width: 1280, height: 800 },
  { width: 1024, height: 1366 },
  { width: 1366, height: 1024 },
];
/** From this width the list and its detail stand side by side. */
const WIDE = 1000;
const RAIL = 88;
const TOUCH = 48;

test.beforeEach(({}, info) => {
  test.skip(info.project.name.startsWith("phone-"), "the tablet tests set their own window size; one run per theme is enough");
});
test.afterAll(async () => {
  await setLayout("bisher");
});

const rail = (page: Page) => page.getByRole("complementary", { name: "Navigation" });
const figure = (page: Page) => page.getByRole("complementary", { name: /: Bild und Chats$/ });
const pane = (page: Page) => page.getByRole("complementary", { name: /^Figur: / });
const box = async (page: Page, selector: string) => (await page.locator(selector).first().boundingBox())!;

async function firstChat(name: string): Promise<{ character: number; chat: number }> {
  const character = await charId(name);
  const { chats } = await api<{ chats: { id: number }[] }>("GET", `/characters/${character}/chats`);
  return { character, chat: chats[0].id };
}

/** Everything a finger can press on the visible screen that is smaller than 48 px in a direction. */
const tooSmall = (page: Page) =>
  page.evaluate((min) => {
    const sel = 'a[href], button, input:not([type="hidden"]), select, textarea, summary, [role="button"], [role="tab"], [role="switch"], [role="slider"]';
    const hits = (el: Element, x: number, y: number) => {
      const t = document.elementFromPoint(x, y);
      return !!t && (t === el || el.contains(t) || !!t.closest("label")?.contains(el));
    };
    const out: string[] = [];
    for (const el of document.querySelectorAll<HTMLElement>(sel)) {
      const st = getComputedStyle(el);
      if (st.visibility === "hidden" || st.display === "none" || st.pointerEvents === "none") continue;
      // A check box or switch inside a label is pressed through the label.
      const r = (el.tagName === "INPUT" ? el.closest("label") : null)?.getBoundingClientRect() ?? el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      // A link in running text is read, not aimed at.
      if (el.tagName === "A" && st.display === "inline") continue;
      const [cx, cy] = [r.left + r.width / 2, r.top + r.height / 2];
      if (cx < 0 || cy < 0 || cx > innerWidth || cy > innerHeight) continue;
      const wide = r.width >= min - 0.5 || (hits(el, cx - (min / 2 - 1), cy) && hits(el, cx + (min / 2 - 1), cy));
      const tall = r.height >= min - 0.5 || (hits(el, cx, cy - (min / 2 - 1)) && hits(el, cx, cy + (min / 2 - 1)));
      if (!wide || !tall) out.push(`${el.tagName.toLowerCase()}.${String(el.className).trim().replace(/\s+/g, ".")} ${Math.round(r.width)}x${Math.round(r.height)}`);
    }
    return out;
  }, TOUCH);

const noSideScroll = (page: Page) => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth);

/** A finger moving sideways over the page, in steps the page sees as real touch input. */
async function swipe(page: Page, x: number, y: number, dx: number) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
  for (let i = 1; i <= 8; i++) {
    await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: x + (dx * i) / 8, y }] });
    await page.waitForTimeout(16);
  }
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await cdp.detach();
}

for (const size of SIZES) {
  const label = `${size.width}x${size.height}`;
  const wide = size.width >= WIDE;
  const sideways = size.width > size.height;

  test.describe(`Tablet ${label}`, () => {
    test.use({ viewport: size, hasTouch: true, isMobile: true });

    test("Navigation: Leiste statt Seitenleiste, jedes Ziel einen Fingertipp entfernt", async ({ page, open }) => {
      await setLayout("bisher");
      await open("/");
      await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
      await expect(page.locator("html")).toHaveAttribute("data-turn", sideways ? "sideways" : "upright");
      await expect(rail(page)).toBeVisible();
      await expect(page.locator("aside.side")).toHaveCount(0);
      await expect(page.locator("header.top")).toBeHidden();
      await expect(page.locator("nav.tabbar")).toBeHidden();
      const r = (await rail(page).boundingBox())!;
      expect(r.x).toBe(0);
      expect(r.width).toBe(RAIL);
      expect(r.height).toBe(size.height);
      // The page starts where the rail ends and never scrolls sideways.
      expect((await box(page, ".view")).x).toBe(RAIL);
      expect(await noSideScroll(page)).toBe(true);

      await expect(rail(page).getByRole("link", { name: "Start", exact: true })).toHaveAttribute("aria-current", "page");
      await rail(page).getByRole("link", { name: "Charaktere" }).tap();
      await expect(page).toHaveURL(/#\/chars$/);
      await expect(rail(page).getByRole("link", { name: "Charaktere" })).toHaveAttribute("aria-current", "page");
      await expect(page.getByRole("heading", { name: "Charaktere", level: 1 })).toBeVisible();
      await rail(page).getByRole("link", { name: "Personas" }).tap();
      await expect(page.getByRole("heading", { name: "Personas", level: 1 })).toBeVisible();
      await rail(page).getByRole("link", { name: "Modelle" }).tap();
      await expect(page).toHaveURL(/#\/settings\/suggestions$/);
      await rail(page).getByRole("link", { name: "Einstellungen" }).tap();
      await expect(page).toHaveURL(/#\/settings$/);
      await expect(rail(page).getByRole("link", { name: "Einstellungen" })).toHaveAttribute("aria-current", "page");
      // The model's state leads to its settings; search opens the quick actions.
      await expect(rail(page).getByRole("status")).toBeAttached();
      await rail(page).locator('a[href="#/settings/model"]').tap();
      await expect(page).toHaveURL(/#\/settings\/model$/);
      await rail(page).getByRole("button", { name: "Suchen und Schnellaktionen" }).tap();
      await expect(page.getByRole("dialog")).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog")).toBeHidden();
      // The back key closes the search window and leaves the page under it where it was.
      await rail(page).getByRole("button", { name: "Suchen und Schnellaktionen" }).tap();
      await expect(page.getByRole("dialog")).toBeVisible();
      await page.goBack();
      await expect(page.getByRole("dialog")).toBeHidden();
      await expect(page).toHaveURL(/#\/settings\/model$/);
      // A page opened from it takes the window's place: back returns to where search was opened.
      await rail(page).getByRole("button", { name: "Suchen und Schnellaktionen" }).tap();
      await page.getByRole("dialog").getByRole("combobox").fill("Logs");
      await page.getByRole("dialog").getByRole("option").first().tap();
      await expect(page.getByRole("dialog")).toBeHidden();
      await expect(page).toHaveURL(/#\/settings\/logs/);
      await page.goBack();
      await expect(page).toHaveURL(/#\/settings\/model$/);
      await expect(page.getByRole("dialog")).toBeHidden();
      // The last chats are the figures' pictures; the open one is marked.
      const mara = await firstChat("Mara Voss");
      await rail(page).getByRole("link", { name: "Chat mit Mara Voss" }).tap();
      await expect(page).toHaveURL(new RegExp(`#/chat/${mara.chat}$`));
      await expect(rail(page).getByRole("link", { name: "Chat mit Mara Voss" })).toHaveAttribute("aria-current", "page");
      // Who you play as: the picture at the foot opens the personas.
      await rail(page).getByRole("button", { name: "Persona wechseln" }).tap();
      await expect(page.getByRole("menu", { name: "Persona wechseln" })).toBeVisible();
      await page.keyboard.press("Escape");
      await rail(page).getByRole("link", { name: "Lorelamp, zum Start" }).tap();
      await expect(page.getByRole("heading", { name: "Start", level: 1 })).toBeVisible();
    });

    test("Charaktere: Karten und Figur nebeneinander", async ({ page, open }) => {
      await open("/chars");
      const ilse = page.getByRole("button", { name: "Kapitän Ilse Brandt: Aktionen zeigen" });
      await expect(ilse).toBeVisible();
      if (wide) {
        // The pane is always there, beside the cards, over the full height.
        await expect(pane(page)).toBeVisible();
        await expect.poll(async () => { const b = (await pane(page).boundingBox())!; return b.x + b.width; }).toBe(size.width);
        const p = (await pane(page).boundingBox())!;
        expect(p.x + p.width).toBe(size.width);
        expect(p.height).toBe(size.height);
        expect((await box(page, ".grid-chars")).x + (await box(page, ".grid-chars")).width).toBeLessThanOrEqual(p.x);
      } else {
        await expect(pane(page)).toBeHidden();
      }
      await ilse.tap();
      await expect(pane(page)).toBeVisible();
      await expect(pane(page)).toHaveAccessibleName("Figur: Kapitän Ilse Brandt");
      await expect(pane(page).getByRole("heading", { name: "Kapitän Ilse Brandt" })).toBeVisible();
      await expect.poll(() => pane(page).locator("img.tb-pane-img").evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);
      // The card's own actions stay off on a tablet: the pane has them.
      await expect(page.locator(".cc-menu")).toHaveCount(0);
      await expect(pane(page).getByRole("navigation", { name: "Letzte Chats mit dieser Figur" }).getByRole("link")).toHaveCount(1);
      await expect(pane(page).getByRole("link", { name: "Karte bearbeiten" })).toBeVisible();
      expect(await noSideScroll(page)).toBe(true);
      if (!wide) {
        // Over the cards: it leaves the way it came, under a finger or by its button.
        await expect.poll(async () => { const b = (await pane(page).boundingBox())!; return b.x + b.width; }).toBe(size.width);
        const p = (await pane(page).boundingBox())!;
        expect(p.x + p.width).toBe(size.width);
        await swipe(page, p.x + 60, p.y + p.height - 80, 220);
        await expect(pane(page)).toBeHidden();
        await ilse.tap();
        await expect(pane(page)).toBeVisible();
        await pane(page).getByRole("button", { name: "Figur schließen" }).tap();
        await expect(pane(page)).toBeHidden();
        await ilse.tap();
      }
      // Another card: the pane follows.
      if (wide) {
        await page.getByRole("button", { name: "Mara Voss: Aktionen zeigen" }).tap();
        await expect(pane(page)).toHaveAccessibleName("Figur: Mara Voss");
        await ilse.tap();
      }
      const { chat } = await firstChat("Kapitän Ilse Brandt");
      await pane(page).getByRole("button", { name: "Weiterspielen" }).tap();
      await expect(page).toHaveURL(new RegExp(`#/chat/${chat}$`));
    });

    test("Einstellungen: Bereiche als Liste neben dem Inhalt", async ({ page, open }) => {
      await open("/settings");
      const tabs = page.getByRole("navigation", { name: "Bereiche der Einstellungen" });
      await expect(tabs.getByRole("link", { name: "Allgemein" })).toHaveAttribute("aria-current", "page");
      const t = (await tabs.boundingBox())!;
      const v = await box(page, ".chat-main > .view");
      if (wide) {
        // Side by side: the list on the left, every area one row.
        expect(t.x + t.width).toBeLessThanOrEqual(v.x);
        expect(v.y).toBe(0);
        const rows = await tabs.getByRole("link").evaluateAll((links) => links.map((a) => Math.round(a.getBoundingClientRect().left)));
        expect(new Set(rows).size).toBe(1);
      } else {
        expect(t.y + t.height).toBeLessThanOrEqual(v.y + 1);
      }
      await tabs.getByRole("link", { name: "Chat", exact: true }).tap();
      await expect(page).toHaveURL(/#\/settings\/chat$/);
      await expect(page.getByRole("heading", { name: "Vorschau" })).toBeVisible();
      await tabs.getByRole("link", { name: "Sicherungen" }).tap();
      await expect(page).toHaveURL(/#\/settings\/backups$/);
      expect(await noSideScroll(page)).toBe(true);
    });

    test("Chat: die Figur hat ihren eigenen Platz, mit Bild und ihren Chats", async ({ page, open }) => {
      const { chat } = await firstChat("Kapitän Ilse Brandt");
      await open(`/chat/${chat}`);
      await expect(figure(page)).toBeVisible();
      await expect(figure(page)).toHaveAccessibleName("Kapitän Ilse Brandt: Bild und Chats");
      await expect.poll(() => figure(page).locator("img.tb-figure-img").evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);
      const f = (await figure(page).boundingBox())!;
      const s = await box(page, ".stream");
      const pic = await box(page, ".tb-figure-pic");
      if (sideways) {
        // A column of its own over the full height, the story beside it.
        expect(f.x).toBe(RAIL);
        expect(f.y).toBe(0);
        expect(f.height).toBe(size.height);
        expect(f.width).toBeGreaterThanOrEqual(272);
        expect(f.width).toBeLessThanOrEqual(372);
        expect(s.x).toBeGreaterThanOrEqual(f.x + f.width);
        expect(pic.width).toBe(f.width - 1);
        expect(pic.height).toBeGreaterThan(size.height * 0.4);
      } else {
        // A head above the story, as wide as the page.
        expect(f.x).toBe(RAIL);
        expect(f.x + f.width).toBe(size.width);
        expect(f.y + f.height).toBeLessThanOrEqual(s.y);
        expect(pic.height).toBe(176);
      }
      // The figure's chats, the open one marked.
      const chats = figure(page).getByRole("navigation", { name: "Chats mit Kapitän Ilse Brandt" });
      await expect(chats.getByRole("link")).toHaveCount(1);
      await expect(chats.getByRole("link").first()).toHaveAttribute("aria-current", "page");
      await expect(figure(page).getByRole("button", { name: "Neuer Chat mit Kapitän Ilse Brandt" })).toBeVisible();
      // The message field stays inside the window, and nothing scrolls sideways.
      const field = (await page.locator(".composer").boundingBox())!;
      expect(field.y + field.height).toBeLessThanOrEqual(size.height);
      expect(await noSideScroll(page)).toBe(true);
      // The key hints of a desk keyboard are gone, and the field is large enough not to zoom.
      await expect(page.locator(".composer .hint")).toBeHidden();
      expect(await page.getByRole("textbox", { name: /^Nachricht an/ }).evaluate((el) => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(16);

      // Away and back: the small picture in the bar brings the figure's place back.
      await figure(page).getByRole("button", { name: "Figur ausblenden" }).tap();
      await expect(figure(page)).toHaveCount(0);
      expect((await box(page, ".stream")).x).toBe(RAIL);
      const back = page.getByRole("button", { name: "Bild von Kapitän Ilse Brandt groß zeigen" });
      await expect(back).toHaveAttribute("aria-pressed", "false");
      await back.tap();
      await expect(figure(page)).toBeVisible();
    });

    test("Chat: ein neuer Chat aus der Figur, und der Wechsel zurück", async ({ page, open }) => {
      const { chat } = await firstChat("Kapitän Ilse Brandt");
      await open(`/chat/${chat}`);
      let fresh: number | undefined;
      try {
        await figure(page).getByRole("button", { name: "Neuer Chat mit Kapitän Ilse Brandt" }).tap();
        await expect(page).not.toHaveURL(new RegExp(`#/chat/${chat}$`));
        fresh = Number(/#\/chat\/(\d+)$/.exec(page.url())?.[1]);
        const chats = figure(page).getByRole("navigation", { name: "Chats mit Kapitän Ilse Brandt" });
        await expect(chats.getByRole("link")).toHaveCount(2);
        await expect(chats.locator('[aria-current="page"]')).toHaveAttribute("href", `#/chat/${fresh}`);
        // List and detail: a tap on the other chat shows it at once, the figure stays where it is.
        await figure(page).evaluate((el) => ((el as HTMLElement & { kept?: boolean }).kept = true));
        await chats.locator(`a[href="#/chat/${chat}"]`).tap();
        await expect(page).toHaveURL(new RegExp(`#/chat/${chat}$`));
        await expect(chats.locator('[aria-current="page"]')).toHaveAttribute("href", `#/chat/${chat}`);
        expect(await figure(page).evaluate((el) => (el as HTMLElement & { kept?: boolean }).kept)).toBe(true);
      } finally {
        if (fresh) await deleteChat(fresh);
      }
    });

    test("Drehen: anderes Layout, nichts geht verloren", async ({ page, open }) => {
      const turned = { width: size.height, height: size.width };
      const { chat } = await firstChat("Kapitän Ilse Brandt");
      await open(`/chat/${chat}`);
      const field = page.getByRole("textbox", { name: /^Nachricht an/ });
      await field.fill("Ein Entwurf, der das Drehen übersteht.");
      type Kept = HTMLElement & { kept?: boolean };
      await page.locator(".chat-main.chat").evaluate((el: Kept) => (el.kept = true));
      await field.evaluate((el: Kept) => (el.kept = true));
      await figure(page).evaluate((el: Kept) => (el.kept = true));
      const before = (await figure(page).boundingBox())!;
      try {
        await page.setViewportSize(turned);
        await expect(page.locator("html")).toHaveAttribute("data-turn", sideways ? "upright" : "sideways");
        await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
        // The same screen, the same field, the same text: only the boxes moved.
        expect(await page.locator(".chat-main.chat").evaluate((el: Kept) => el.kept)).toBe(true);
        expect(await field.evaluate((el: Kept) => el.kept)).toBe(true);
        expect(await figure(page).evaluate((el: Kept) => el.kept)).toBe(true);
        await expect(field).toHaveValue("Ein Entwurf, der das Drehen übersteht.");
        const after = (await figure(page).boundingBox())!;
        const s = await box(page, ".stream");
        if (sideways) {
          // Was a column, is a head now.
          expect(before.height).toBe(size.height);
          expect(after.y + after.height).toBeLessThanOrEqual(s.y);
        } else {
          expect(after.height).toBe(turned.height);
          expect(s.x).toBeGreaterThanOrEqual(after.x + after.width);
        }
        expect((await rail(page).boundingBox())!.height).toBe(turned.height);
        expect(await noSideScroll(page)).toBe(true);
        await page.setViewportSize(size);
        await expect(page.locator("html")).toHaveAttribute("data-turn", sideways ? "sideways" : "upright");
        await expect(field).toHaveValue("Ein Entwurf, der das Drehen übersteht.");
      } finally {
        await page.setViewportSize(size);
        await field.fill("");
      }

      // The characters: what was searched for and the figure on show stay.
      await open("/chars");
      await page.getByRole("button", { name: "Kapitän Ilse Brandt: Aktionen zeigen" }).tap();
      await expect(pane(page)).toHaveAccessibleName("Figur: Kapitän Ilse Brandt");
      const search = page.getByRole("searchbox").or(page.getByPlaceholder("Name, Kurztext, Tag")).first();
      await search.fill("a");
      await page.setViewportSize(turned);
      await expect(search).toHaveValue("a");
      if (turned.width >= WIDE) await expect(pane(page)).toHaveAccessibleName("Figur: Kapitän Ilse Brandt");
      expect(await noSideScroll(page)).toBe(true);
      await page.setViewportSize(size);
      await expect(search).toHaveValue("a");

      // The settings keep their area.
      await open("/settings/chat");
      await page.setViewportSize(turned);
      await expect(page.getByRole("navigation", { name: "Bereiche der Einstellungen" }).getByRole("link", { name: "Chat", exact: true })).toHaveAttribute("aria-current", "page");
      await expect(page.getByRole("heading", { name: "Vorschau" })).toBeVisible();
      expect(await noSideScroll(page)).toBe(true);
    });

    test("Bildschirmtastatur: das Layout springt nicht, Feld und Geschichte bleiben", async ({ page, open }) => {
      const { chat } = await firstChat("Kapitän Ilse Brandt");
      await open(`/chat/${chat}`);
      const turn = sideways ? "sideways" : "upright";
      await expect(page.locator("html")).toHaveAttribute("data-turn", turn);
      // The keyboard takes about 42 % of the height; an upright window is then wider than tall.
      const typing = { width: size.width, height: Math.round(size.height * 0.58) };
      await page.setViewportSize(typing);
      await expect(page.locator("html")).toHaveAttribute("data-turn", turn);
      const field = (await page.locator(".composer").boundingBox())!;
      expect(field.y + field.height).toBeLessThanOrEqual(typing.height);
      const s = await box(page, ".stream");
      expect(s.height).toBeGreaterThanOrEqual(160);
      if (sideways) {
        // The picture stays beside the story.
        await expect(figure(page)).toBeVisible();
        expect((await figure(page).boundingBox())!.height).toBe(typing.height);
      } else if (typing.height <= 820) {
        // The head makes room for the story and comes back with the keyboard gone.
        await expect(figure(page)).toBeHidden();
      }
      await page.setViewportSize(size);
      await expect(figure(page)).toBeVisible();
    });

    test("Touch-Ziele: mindestens 48 px", async ({ page, open }) => {
      const { character, chat } = await firstChat("Mara Voss");
      for (const layout of ["bisher", "dashboard", "bento", "buehne", "kommando"]) {
        await setLayout(layout);
        await open("/");
        expect(await tooSmall(page), `Start (${layout})`).toEqual([]);
      }
      for (const hash of ["/chars", `/chat/${chat}`, `/char/${character}/chats`, "/settings", "/settings/chat", "/personas"]) {
        await open(hash);
        expect(await tooSmall(page), hash).toEqual([]);
      }
    });

    test("Start: alle fünf Layouts passen in die Breite", async ({ page, open }) => {
      for (const layout of ["bisher", "dashboard", "bento", "buehne", "kommando"]) {
        await setLayout(layout);
        await open("/");
        await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
        expect(await noSideScroll(page), layout).toBe(true);
        // Nothing of the page lies under the rail or runs out of the window.
        const overflow = await page.evaluate((left) => {
          const view = document.querySelector(".view")!;
          const out: string[] = [];
          for (const el of view.querySelectorAll<HTMLElement>(".page > *, .page section, .page article")) {
            const r = el.getBoundingClientRect();
            if (r.width > 0 && (r.left < left - 0.5 || r.right > innerWidth + 0.5)) out.push(`${el.className} ${Math.round(r.left)}–${Math.round(r.right)}`);
          }
          return out;
        }, RAIL);
        expect(overflow, layout).toEqual([]);
      }
    });

    test("Bildschirmfotos", async ({ page, open, theme }) => {
      test.skip(!shots, "review screenshots only with RP_E2E_SHOTS");
      mkdirSync(shots!, { recursive: true });
      const shot = async (name: string) => {
        await page.evaluate(() => document.fonts.ready);
        await page.waitForTimeout(450); // panes and sheets have settled
        await page.screenshot({ path: join(shots!, `${name}-${label}-${theme}.png`) });
      };
      const ilse = await firstChat("Kapitän Ilse Brandt");
      const mara = await firstChat("Mara Voss");
      for (const layout of ["bisher", "dashboard", "bento", "buehne", "kommando"]) {
        await setLayout(layout);
        await open("/");
        await shot(`start-${layout}`);
      }
      await setLayout("bisher");
      await open("/chars");
      await shot("charaktere");
      await page.getByRole("button", { name: "Kapitän Ilse Brandt: Aktionen zeigen" }).tap();
      await shot("charaktere-figur");
      await open(`/chat/${ilse.chat}`);
      await shot("chat-bild");
      await page.setViewportSize({ width: size.width, height: Math.round(size.height * 0.58) });
      await shot("chat-tastatur");
      await page.setViewportSize(size);
      await page.getByRole("button", { name: "Aussehen dieses Chats" }).first().tap();
      await shot("chat-aussehen");
      await open(`/chat/${mara.chat}`);
      await shot("chat-ohne-bild");
      await open(`/char/${mara.character}/chats`);
      await shot("figur-chats");
      await open("/settings");
      await shot("einstellungen");
      await open("/settings/chat");
      await shot("einstellungen-chat");
      await open("/personas");
      await shot("personas");
    });
  });
}

test.describe("Tablet: Varianten wischen", () => {
  test.use({ viewport: { width: 1280, height: 800 }, hasTouch: true, isMobile: true });
  const stFixture = (name: string) => readFileSync(fileURLToPath(new URL(`../../../tests/fixtures/sillytavern/${name}`, import.meta.url)));
  const THEO_CHAT = "Theo/Theo - 2026-03-01@18h30m00s.jsonl";
  type StResult = { file: string; status: string; character_id?: number; chat_id?: number };

  test("ein Wisch nach links zeigt die nächste Variante, nach rechts die vorige", async ({ page, open }) => {
    test.skip(!stRoot, "needs RP_ST_SEARCH_ROOTS on the test server and the same folder in RP_E2E_ST_ROOT");
    const install = join(stRoot!, `SillyTavern-tb-${Date.now().toString(36)}`);
    const user = join(install, "data", "default-user");
    mkdirSync(join(user, "characters"), { recursive: true });
    mkdirSync(join(user, "chats", "Theo"), { recursive: true });
    writeFileSync(join(user, "characters", "Theo.json"), stFixture("theo_lindqvist.json"));
    writeFileSync(join(user, "chats", THEO_CHAT), stFixture("chat_theo.jsonl"));
    let chatId: number | undefined;
    let newCharacter: number | undefined;
    try {
      const take = () => api<{ results: StResult[] }>("POST", "/import/sillytavern", { path: install, characters: ["Theo.json"], chats: [THEO_CHAT] });
      let r = await take();
      const card = r.results.find((x) => x.file === "characters/Theo.json");
      if (card?.status === "imported") newCharacter = card.character_id;
      let chat = r.results.find((x) => x.file === `chats/${THEO_CHAT}`);
      if (chat?.status === "duplicate" && chat.chat_id) {
        await deleteChat(chat.chat_id);
        r = await take();
        chat = r.results.find((x) => x.file === `chats/${THEO_CHAT}`);
      }
      expect(chat?.status, JSON.stringify(r)).toBe("imported");
      chatId = chat!.chat_id!;

      await open(`/chat/${chatId}`);
      const all = page.getByRole("article");
      await expect(all).toHaveCount(5);
      const reply = page.locator(`[data-msg="${await all.nth(2).getAttribute("data-msg")}"]`);
      const variants = reply.getByRole("group", { name: "Antwort-Varianten" });
      const prose = reply.locator(".prose");
      await expect(variants).toContainText("2 / 3");
      const at = async () => {
        const b = (await prose.boundingBox())!;
        return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
      };
      // A short way is not a swipe: the text settles back, the variant stays.
      let p = await at();
      await swipe(page, p.x, p.y, -30);
      await expect(variants).toContainText("2 / 3");
      // To the left: the next one. At the last one there is nowhere further.
      await swipe(page, p.x, p.y, -160);
      await expect(variants).toContainText("3 / 3");
      p = await at();
      await swipe(page, p.x, p.y, -160);
      await expect(variants).toContainText("3 / 3");
      // To the right: back.
      await swipe(page, p.x, p.y, 160);
      await expect(variants).toContainText("2 / 3");
      p = await at();
      await swipe(page, p.x, p.y, 160);
      await expect(variants).toContainText("1 / 3");
      // The text is back in its place.
      await expect.poll(() => prose.evaluate((el) => getComputedStyle(el).transform)).toMatch(/^(none|matrix\(1, 0, 0, 1, 0, 0\))$/);
      // The choice is the server's: it is there after a reload.
      await page.reload();
      await expect(reply.getByRole("group", { name: "Antwort-Varianten" })).toContainText("1 / 3");
    } finally {
      if (chatId) await deleteChat(chatId).catch(() => {});
      if (newCharacter) await api("DELETE", `/characters/${newCharacter}`).catch(() => {});
      rmSync(install, { recursive: true, force: true });
    }
  });
});

test.describe("Formfaktor", () => {
  test.describe("Maus am PC", () => {
    test.use({ viewport: { width: 1280, height: 800 }, hasTouch: false, isMobile: false });

    test("ein Fenster in Tablet-Breite bleibt der Desktop", async ({ page, open }) => {
      await open("/");
      await expect(page.locator("html")).toHaveAttribute("data-form", "desktop");
      await expect(page.locator("aside.side")).toBeVisible();
      await expect(rail(page)).toHaveCount(0);
    });

    test("die Android-App (shell=android) bekommt das Tablet-Layout", async ({ page, open }) => {
      await open("/"); // puts the project's theme in place
      await page.goto(`/?token=${token}&shell=android#/`);
      await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
      await expect(rail(page)).toBeVisible();
      await expect(page.locator("aside.side")).toHaveCount(0);
      // Kept for the tab: a reload without the parameter stays in the app's layout.
      await page.goto(`/?token=${token}#/chars`);
      await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
      // Wider than a tablet: the desktop again.
      await page.setViewportSize({ width: 1600, height: 900 });
      await expect(page.locator("html")).toHaveAttribute("data-form", "desktop");
      await expect(page.locator("aside.side")).toBeVisible();
    });
  });

  test.describe("Finger", () => {
    test.use({ viewport: { width: 700, height: 1100 }, hasTouch: true, isMobile: true });

    test("unter 768 px ist es das Handy, darüber das Tablet", async ({ page, open }) => {
      await open("/");
      await expect(page.locator("html")).toHaveAttribute("data-form", "phone");
      await expect(page.locator("header.top")).toBeVisible();
      await expect(rail(page)).toHaveCount(0);
      // A small tablet turned sideways crosses the line: the page stays, the navigation changes.
      await page.locator(".view").evaluate((el: HTMLElement & { kept?: boolean }) => (el.kept = true));
      await page.setViewportSize({ width: 1100, height: 700 });
      await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
      await expect(rail(page)).toBeVisible();
      expect(await page.locator(".view").evaluate((el: HTMLElement & { kept?: boolean }) => el.kept)).toBe(true);
    });
  });
});
