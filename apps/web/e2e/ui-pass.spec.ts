// Whole-app layout checks from the full UI pass (step 6): no page scrolls sideways, and the bugs the
// pass found stay fixed (class names shared by two screens, long words, tabs out of view, small targets).

import { api, baseURL, charId, deleteChat, newChat, setLayout, setTheme, token } from "./api";
import { messageIds } from "./db";
import { expect, isPhone, test } from "./fixtures";
import type { Page } from "@playwright/test";

test.afterAll(async () => {
  await setLayout("bisher");
});

/** Every main scroll area of the page fits its width: nothing in it pushes it sideways. */
async function noSideways(page: Page, where: string) {
  const wide = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>(".view, .chat-main, .app")]
      .filter((el) => el.scrollWidth > el.clientWidth + 1)
      .map((el) => `${el.className} ${el.scrollWidth}/${el.clientWidth}`),
  );
  expect(wide, `${where} scrolls sideways`).toEqual([]);
}

test("keine Seite scrollt seitwärts", async ({ page, open }) => {
  const mara = await charId("Mara Voss");
  const { chats } = await api<{ chats: { id: number }[] }>("GET", `/characters/${mara}/chats`);
  const pages = [
    "/chars", "/import", `/char/${mara}/chats`, `/char/${mara}/settings`, `/chat/${chats[0].id}`, `/chat/${chats[0].id}/card`,
    "/settings", "/settings/model", "/settings/expert", "/settings/suggestions", "/settings/backups", "/settings/logs", "/personas", "/personas/new",
  ];
  for (const hash of pages) {
    await open(hash);
    await page.waitForTimeout(300);
    await noSideways(page, hash);
  }
});

test("Einstellungen: Schalter-Texte brechen um und bleiben im Feld", async ({ page, open }) => {
  await open("/settings");
  const row = page.locator('[data-spot="answer-hint"] .switch-row').first();
  await expect(row).toBeVisible();
  const [panel, label] = await Promise.all([page.locator('[data-spot="answer-hint"]').boundingBox(), row.boundingBox()]);
  expect(label!.x + label!.width).toBeLessThanOrEqual(panel!.x + panel!.width);
  // The setting's text is not the start screen's one-line "Neuer Chat mit …" row.
  await expect(row.locator(".sw-text")).toHaveCSS("white-space", "normal");
});

test("Logs: Millisekunden stehen in derselben Zeile wie die Uhrzeit", async ({ page, open }) => {
  await open("/settings/logs");
  const time = page.locator(".log-main time").first();
  await expect(time).toBeVisible();
  const box = await time.boundingBox();
  const line = await time.evaluate((el) => parseFloat(getComputedStyle(el).lineHeight) || 20);
  expect(box!.height).toBeLessThan(line * 1.5);
});

test("Einstellungen: der offene Tab ist in der Tab-Leiste zu sehen", async ({ page, open }) => {
  await open("/settings/logs");
  const nav = page.getByRole("navigation", { name: "Bereiche der Einstellungen" });
  const tab = nav.getByRole("link", { name: "Logs" });
  await expect(tab).toHaveAttribute("aria-current", "page");
  await expect(tab).toBeInViewport({ ratio: 1 });
  // The title keeps the page's side margin (no back button on this page; it sat at 4 px on a phone).
  const title = await page.getByRole("heading", { name: "Einstellungen", level: 1 }).boundingBox();
  expect(title!.x).toBeGreaterThanOrEqual(12);
});

test("Start: ein langes Wort in der letzten Nachricht bricht in jedem Layout um", async ({ page, open }) => {
  const mara = await charId("Mara Voss");
  const chat = await newChat(mara);
  try {
    const [greeting] = messageIds(chat);
    await api("PUT", `/messages/${greeting}`, { content: `Ein Hinweis: ${"langeswortohneleerzeichen".repeat(6)} und weiter.` });
    for (const layout of ["bisher", "dashboard", "bento", "buehne", "kommando"]) {
      await setLayout(layout);
      await open("/");
      await expect(page.getByText(/langeswortohneleerzeichen/).first()).toBeVisible();
      const spill = await page.evaluate(() =>
        [...document.querySelectorAll<HTMLElement>("blockquote, .rc-quote, .cmd-lead-quote, .fig-blurb, .cmd-fig-blurb")]
          .filter((el) => el.textContent?.includes("langeswortohneleerzeichen") && el.scrollWidth > el.clientWidth + 1)
          .map((el) => el.className || el.tagName),
      );
      expect(spill, layout).toEqual([]);
      await noSideways(page, `Start ${layout}`);
    }
  } finally {
    await deleteChat(chat);
  }
});

test("Figur: eigene Tags sind am Handy groß genug zum Tippen", async ({ page, browser, theme }) => {
  test.skip(!isPhone(page), "touch sizes only matter on the phone");
  const mara = await charId("Mara Voss");
  await api("PUT", `/characters/${mara}/tags`, { tags: ["Hafen"] });
  // A touch screen (pointer: coarse), as on a real phone.
  const ctx = await browser.newContext({ viewport: page.viewportSize()!, hasTouch: true, locale: "de-DE", baseURL });
  try {
    await setTheme(theme);
    const touch = await ctx.newPage();
    await touch.goto(`/?token=${token}#/char/${mara}/settings`);
    const x = touch.getByRole("button", { name: "Tag Hafen entfernen" });
    await x.scrollIntoViewIfNeeded();
    const box = await x.boundingBox();
    expect(box!.width).toBeGreaterThanOrEqual(44);
    expect(box!.height).toBeGreaterThanOrEqual(44);
  } finally {
    await ctx.close();
    await api("PUT", `/characters/${mara}/tags`, { tags: [] });
  }
});

test("Vorschläge: am PC stehen die drei Spalten nebeneinander, die Handy-Tabs sind weg", async ({ page, open }) => {
  await open("/settings/suggestions");
  const tabs = page.getByRole("button", { name: "Kontext", exact: true });
  if (isPhone(page)) {
    // The phone shows one column at a time; the tab switches to it.
    await tabs.click();
    await expect(tabs).toHaveAttribute("aria-pressed", "true");
  } else {
    await expect(tabs).toBeHidden();
    await expect(page.getByRole("heading", { name: "Kontext", level: 3, exact: true })).toBeVisible();
  }
});
