// X3 picking cards in the characters tab without aiming at each checkbox: a frame dragged with the
// mouse picks every card it touches (in all three layouts, Esc cancels it, it scrolls along at the edge),
// Shift+click picks a range, Ctrl+A every card on screen (only the filtered ones) and a long press with a
// finger starts the selection with that card. Six SFW test cards of its own join the three fixtures.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { api, library } from "./api";
import { expect, test } from "./fixtures";

const PREFIX = "Rahmenprobe";
const EXTRA = 6;
const TAG = "Auswahltest";
const shots = process.env.RP_E2E_SHOTS;

const status = (page: Page) => page.getByRole("toolbar", { name: "Auswahl" }).getByRole("status");
const names = (page: Page) => page.locator(".grid-chars .cc .body b");
const pickedNames = (page: Page) => page.locator(".grid-chars .cc.picked .body b");
const cards = (page: Page) => page.locator(".grid-chars .cc[data-id]");

async function shot(page: Page, name: string) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  await page.waitForTimeout(300); // the checkboxes' own transition
  await page.screenshot({ path: join(shots, `x3-${name}-${test.info().project.name}.png`) });
}

const cardJson = (name: string) =>
  JSON.stringify({
    spec: "chara_card_v2",
    spec_version: "2.0",
    data: { name, description: "Eine Testfigur für die Auswahl.", personality: "ruhig", scenario: "Ein Hafen.", first_mes: "Hallo.", mes_example: "", tags: [TAG] },
  });

/** How many cards of its own the spec adds (at least six). */
let own = 0;

// Every layout needs a frame that touches exactly five cards. Rows of two (phone poster) and three
// (phone compact) only allow it when the last row is not full, so the total is one less than a
// multiple of six.
test.beforeAll(async () => {
  const all = (await library()).characters;
  const base = all.filter((c) => !c.name.startsWith(PREFIX)).length;
  own = EXTRA;
  while ((base + own) % 6 !== 5) own += 1;
  const have = new Set(all.map((c) => c.name));
  for (let i = 1; i <= own; i++) {
    const name = `${PREFIX} ${i}`;
    if (!have.has(name)) await api("POST", "/characters/import", new TextEncoder().encode(cardJson(name)));
  }
});

test.afterAll(async () => {
  for (const c of (await library()).characters) if (c.name.startsWith(PREFIX)) await api("DELETE", `/characters/${c.id}`);
  await api("PUT", "/settings", { library_layout: "poster" });
});

async function startSelecting(page: Page) {
  await page.getByRole("button", { name: "Auswählen", exact: true }).click();
  await expect(status(page)).toHaveText("Nichts ausgewählt");
}

/** `end`: the card the frame ends in (its id). */
type Plan = { from: { x: number; y: number }; to: { x: number; y: number }; end: string; expected: string[] };

/**
 * Where the frame ends, measured again once it runs: on a phone the selection bar gets shorter with its
 * first pick, and at the end of the page the cards then move down under the pointer.
 */
async function endPoint(page: Page, plan: Plan) {
  const b = (await page.locator(`.grid-chars .cc[data-id="${plan.end}"]`).boundingBox())!;
  return { x: b.x + b.width * 0.7, y: b.y + 12 };
}

/**
 * A frame that touches exactly five cards: from inside one card to inside another, short enough to stay
 * clear of the edges where the page scrolls. The page is scrolled so both points are on screen.
 */
async function planFrame(page: Page): Promise<Plan> {
  const plan = await page.evaluate(() => {
    const els = [...document.querySelectorAll<HTMLElement>(".grid-chars .cc[data-id]")];
    const view = els[0].closest<HTMLElement>(".view")!;
    const viewBox = view.getBoundingClientRect();
    const bar = document.querySelector(".lib-selbar")!.getBoundingClientRect();
    const edge = 76; // the scrolling band (56 px) and some room
    const room = Math.min(viewBox.bottom, bar.top) - viewBox.top - 2 * edge;
    const boxes = els.map((el) => el.getBoundingClientRect());
    for (let i = 0; i < els.length; i++) {
      for (let j = i + 1; j < els.length; j++) {
        const a = boxes[i];
        const b = boxes[j];
        const from = { x: a.left + a.width * 0.3, y: a.bottom - 12 };
        const to = { x: b.left + b.width * 0.7, y: b.top + 12 };
        const m = { left: Math.min(from.x, to.x), right: Math.max(from.x, to.x), top: Math.min(from.y, to.y), bottom: Math.max(from.y, to.y) };
        if (m.bottom - m.top > room) continue;
        const hit = boxes.map((r, k) => (r.left < m.right && r.right > m.left && r.top < m.bottom && r.bottom > m.top ? k : -1)).filter((k) => k >= 0);
        if (hit.length !== 5) continue;
        // Scroll so the frame starts just below the top band.
        view.scrollTop += m.top - (viewBox.top + edge);
        const shift = a.bottom - els[i].getBoundingClientRect().bottom;
        return {
          from: { x: from.x, y: from.y - shift },
          to: { x: to.x, y: to.y - shift },
          end: els[j].dataset.id ?? "",
          expected: hit.map((k) => els[k].querySelector(".body b")!.textContent ?? ""),
        };
      }
    }
    return null;
  });
  if (!plan) throw new Error("no frame over exactly five cards found");
  return plan;
}

for (const layout of ["poster", "compact", "list"] as const) {
  test(`Auswahl: Rahmen über fünf Karten wählt genau fünf (${layout})`, async ({ page, open }) => {
    await api("PUT", "/settings", { library_layout: layout });
    await open("/chars");
    await expect(page.locator(`.grid-chars.lib-${layout}`)).toBeVisible();
    await expect(cards(page)).toHaveCount((await library()).characters.length);
    await startSelecting(page);

    const plan = await planFrame(page);
    const { from, expected } = plan;
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    // Under the threshold nothing happens yet.
    await page.mouse.move(from.x + 3, from.y + 2);
    await expect(page.locator(".x3-frame")).toBeHidden();
    await page.mouse.move(from.x + 12, from.y + 8, { steps: 3 });
    await expect(page.locator(".x3-frame")).toBeVisible();
    await expect(status(page)).toHaveText("1 ausgewählt");
    const to = await endPoint(page, plan);
    await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 6 });
    await page.mouse.move(to.x, to.y, { steps: 6 });
    await expect(page.locator(".x3-frame")).toBeVisible();
    await expect(status(page)).toHaveText("5 ausgewählt");
    await shot(page, `rahmen-${layout}`);
    await page.mouse.up();

    // The click that ends the frame does not unpick the card under the mouse.
    await expect(page.locator(".x3-frame")).toBeHidden();
    await expect(status(page)).toHaveText("5 ausgewählt");
    await expect(pickedNames(page)).toHaveText(expected);
    expect(await page.evaluate(() => getSelection()?.toString() ?? "")).toBe("");
    await shot(page, `gewaehlt-${layout}`);
  });
}

test("Auswahl: Esc bricht den Rahmen ab, ein Klick ohne Ziehen wählt wie bisher", async ({ page, open }) => {
  await open("/chars");
  await startSelecting(page);
  const first = cards(page).first();
  await first.click();
  await expect(status(page)).toHaveText("1 ausgewählt");

  const { from, to } = await planFrame(page);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 8 });
  await expect(page.locator(".x3-frame")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".x3-frame")).toBeHidden();
  await expect(status(page)).toHaveText("1 ausgewählt");
  await page.mouse.up();
  await expect(status(page)).toHaveText("1 ausgewählt");
  await expect(first).toHaveClass(/\bpicked\b/);
  // Esc on its own did not leave the selection mode.
  await expect(page.getByRole("toolbar", { name: "Auswahl" })).toBeVisible();
});

test("Auswahl: Rahmen am Rand scrollt mit", async ({ page, open }) => {
  const width = page.viewportSize()!.width;
  await page.setViewportSize({ width, height: 620 });
  await open("/chars");
  await startSelecting(page);
  const view = page.locator(".view").filter({ has: page.locator(".grid-chars") });
  const first = (await cards(page).first().boundingBox())!;
  await page.mouse.move(first.x + first.width / 2, first.y + 20);
  await page.mouse.down();
  await page.mouse.move(first.x + first.width / 2 + 20, first.y + 40, { steps: 3 });
  await expect(status(page)).toHaveText("1 ausgewählt");
  // Across all columns into the band above the selection bar (measured now: its text got shorter), and
  // the page scrolls down to its end on its own.
  const bar = (await page.locator(".lib-selbar").boundingBox())!;
  await page.mouse.move(bar.x + bar.width - 30, bar.y - 10, { steps: 8 });
  await expect.poll(() => view.evaluate((el) => el.scrollHeight - el.clientHeight - el.scrollTop), { timeout: 15_000 }).toBeLessThan(2);
  await expect(page.locator(".x3-frame")).toBeVisible();
  await page.mouse.up();
  // The last card was far below the screen when the frame began.
  await expect(cards(page).last()).toHaveClass(/\bpicked\b/);
  await expect(cards(page).first()).toHaveClass(/\bpicked\b/);
});

test("Auswahl: Umschalt+Klick wählt den Bereich", async ({ page, open }) => {
  await open("/chars");
  await startSelecting(page);
  const all = await names(page).allTextContents();
  await cards(page).nth(1).click();
  await cards(page).nth(5).click({ modifiers: ["Shift"] });
  await expect(status(page)).toHaveText("5 ausgewählt");
  await expect(pickedNames(page)).toHaveText(all.slice(1, 6));
  // On from the card clicked last; the first range stays.
  await cards(page).nth(7).click({ modifiers: ["Shift"] });
  await expect(pickedNames(page)).toHaveText(all.slice(1, 8));
  expect(await page.evaluate(() => getSelection()?.toString() ?? "")).toBe("");
});

test("Auswahl: Strg+A wählt alle sichtbaren, mit Filter nur die gefilterten", async ({ page, open }) => {
  const total = (await library()).characters.length;
  await open("/chars");
  // Also starts the selection mode.
  await page.keyboard.press("Control+a");
  await expect(status(page)).toHaveText(`${total} ausgewählt`);
  expect(await page.evaluate(() => getSelection()?.toString() ?? "")).toBe("");

  // In the search field Ctrl+A stays the field's.
  const search = page.getByRole("textbox", { name: "Charaktere durchsuchen" });
  await search.fill("Brandt");
  await search.press("Control+a");
  expect(await search.evaluate((el: HTMLInputElement) => (el.selectionEnd ?? 0) - (el.selectionStart ?? 0))).toBe("Brandt".length);
  await expect(status(page)).toHaveText(`${total} ausgewählt`);
  await search.fill("");

  // With the tag filter on: only the filtered cards.
  await page.getByRole("button", { name: "Keine auswählen" }).click();
  await page
    .getByRole("group", { name: "Nach Tag filtern" })
    .getByRole("button", { name: new RegExp(`^${TAG}`) })
    .click();
  await expect(names(page)).toHaveCount(own);
  await page.keyboard.press("Control+a");
  await expect(status(page)).toHaveText(`${own} ausgewählt`);
  await expect(pickedNames(page)).toHaveText(Array.from({ length: own }, (_, i) => `${PREFIX} ${i + 1}`).sort((a, b) => a.localeCompare(b, "de", { sensitivity: "base" })));
  await shot(page, "strg-a-filter");
});

test.describe("Touch", () => {
  test.use({ hasTouch: true });

  test("Auswahl: langes Drücken wählt die Karte", async ({ page, open }) => {
    await open("/chars");
    const target = cards(page).filter({ hasText: `${PREFIX} 2` });
    await target.scrollIntoViewIfNeeded();
    const box = (await target.boundingBox())!;
    const point = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [point] });
    // The selection starts while the finger still rests on the card.
    await expect(status(page)).toHaveText("1 ausgewählt");
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await page.waitForTimeout(400);
    await expect(status(page)).toHaveText("1 ausgewählt");
    await expect(pickedNames(page)).toHaveText([`${PREFIX} 2`]);
    await expect(page.getByRole("group", { name: /^Aktionen für/ })).toHaveCount(0);

    // A short tap picks another card as before.
    await cards(page).filter({ hasText: `${PREFIX} 3` }).tap();
    await expect(status(page)).toHaveText("2 ausgewählt");
    await shot(page, "lang-gedrueckt");
  });
});
