// The tab "Vorschläge" with a real mouse wheel: on a wide screen the filter column and the card list
// are each their own scroll window, so the wheel over them scrolls them and the page stays where it is
// (the filter's last field must come into sight; it used to sit below the view's bottom edge while the
// wheel did nothing). Beside them the wheel still scrolls the page. On a phone the page scrolls as a
// whole and everything down to the last section is reachable.
// The app's page scroller is the tab's `.view` (the window itself never scrolls), so both are checked.

import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { expect, isPhone, test } from "./fixtures";

const SHOTS = process.env.RP_E2E_SHOTS; // folder for review screenshots (optional)

const scrollTop = (l: Locator) => l.evaluate((e) => Math.round(e.scrollTop));
const pageScroll = (page: Page) => page.evaluate(() => ({ win: window.scrollY, view: Math.round(document.querySelector(".view")!.scrollTop) }));
const wheelOver = async (page: Page, l: Locator, dy: number) => {
  const b = (await l.boundingBox())!;
  await page.mouse.move(b.x + b.width / 2, b.y + Math.min(b.height / 2, 300));
  await page.mouse.wheel(0, dy);
};
const box = async (l: Locator) => (await l.boundingBox())!;

async function openCatalog(page: Page, open: (hash: string) => Promise<void>) {
  await open("/settings/suggestions");
  const list = page.getByRole("list", { name: "Vorschläge", exact: true });
  await expect(list.getByRole("listitem").first()).toBeVisible();
  return list;
}

test("Vorschläge: Mausrad scrollt Filter und Liste, die Seite bleibt stehen", async ({ page, open }, info) => {
  test.skip(isPhone(page), "wide screens only");
  const list = await openCatalog(page, open);
  const toggle = page.getByRole("button", { name: /^Filter/ }).first();
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  const side = page.getByRole("complementary", { name: "Filter", exact: true });
  await expect(side).toBeVisible();

  // Bring the catalog panel to the top of the view: the whole panel (tool bar, filter, list) fits.
  const view = page.locator(".view");
  const panel = page.locator('[data-spot="suggestions"]');
  await panel.evaluate((e) => e.scrollIntoView({ block: "start" }));
  const v = await box(view);
  const p = await box(panel);
  expect(p.y).toBeGreaterThanOrEqual(v.y - 1);
  expect(p.y + p.height).toBeLessThanOrEqual(v.y + v.height + 1);
  const before = await pageScroll(page);

  // The filter column: taller than its window at 1920×1080, so the wheel scrolls it to its end and its
  // last field ends up fully in sight.
  const [sh, ch] = await side.evaluate((e) => [e.scrollHeight, e.clientHeight]);
  if (sh > ch + 1) {
    await wheelOver(page, side, 300);
    await expect.poll(() => scrollTop(side)).toBeGreaterThan(0);
    for (let i = 0; i < 10 && (await side.evaluate((e) => e.scrollTop + e.clientHeight < e.scrollHeight - 1)); i++) await wheelOver(page, side, 300);
    await expect.poll(() => side.evaluate((e) => e.scrollTop + e.clientHeight >= e.scrollHeight - 1)).toBe(true);
    const last = side.getByRole("group", { name: "Stufe" });
    const l = await box(last);
    const s = await box(side);
    expect(l.y + l.height).toBeLessThanOrEqual(Math.min(s.y + s.height, v.y + v.height) + 1);
    expect(await pageScroll(page)).toEqual(before);
  }

  // The card list scrolls on its own.
  const [lsh, lch] = await list.evaluate((e) => [e.scrollHeight, e.clientHeight]);
  expect(lsh).toBeGreaterThan(lch);
  await wheelOver(page, list, 400);
  await expect.poll(() => scrollTop(list)).toBeGreaterThan(0);
  await wheelOver(page, list, 400);
  await expect.poll(() => scrollTop(list)).toBeGreaterThan(400);
  expect(await pageScroll(page)).toEqual(before);
  if (SHOTS) await page.screenshot({ path: join(SHOTS, `x2-scroll-${info.project.name}.png`), animations: "disabled" });

  // Beside the two windows (over the section's text) the wheel still scrolls the page.
  await wheelOver(page, page.getByRole("heading", { name: "Vorschläge", exact: true }), 300);
  await expect.poll(async () => (await pageScroll(page)).view).toBeGreaterThan(before.view);
  expect((await pageScroll(page)).win).toBe(0);
});

test("Vorschläge am Handy: die Seite scrollt als Ganzes, alles ist erreichbar", async ({ page, open }, info) => {
  test.skip(!isPhone(page), "phone only");
  const list = await openCatalog(page, open);
  // No scroll window of its own: the list is as tall as its cards.
  const [sh, ch] = await list.evaluate((e) => [e.scrollHeight, e.clientHeight]);
  expect(sh).toBeLessThanOrEqual(ch + 1);

  const view = page.locator(".view");
  const start = (await pageScroll(page)).view;
  await list.getByRole("listitem").first().scrollIntoViewIfNeeded();
  const before = (await pageScroll(page)).view;
  await wheelOver(page, list.getByRole("listitem").first(), 400);
  await expect.poll(async () => (await pageScroll(page)).view).toBeGreaterThan(before);
  expect(before).toBeGreaterThanOrEqual(start);
  if (SHOTS) await page.screenshot({ path: join(SHOTS, `x2-scroll-${info.project.name}.png`), animations: "disabled" });

  // Wheel on to the end of the page: the last card and the last section come into sight.
  for (let i = 0; i < 80 && (await view.evaluate((e) => e.scrollTop + e.clientHeight < e.scrollHeight - 1)); i++) {
    await page.mouse.wheel(0, 1200);
    await page.waitForTimeout(30);
  }
  await expect.poll(() => view.evaluate((e) => e.scrollTop + e.clientHeight >= e.scrollHeight - 1)).toBe(true);
  await expect(page.getByRole("heading", { name: "Hugging-Face-Link einfügen" })).toBeInViewport();
  const lastCard = list.getByRole("listitem").last();
  await lastCard.scrollIntoViewIfNeeded();
  await expect(lastCard).toBeInViewport();
  expect((await pageScroll(page)).win).toBe(0);
});
