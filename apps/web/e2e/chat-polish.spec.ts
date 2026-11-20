// The chat's small things (audit W1-B): the measurements under a reply are never cut, the key hints
// under the message field stay one line beside the "Aussehen" panel, the panel never opens blank,
// and narrow columns fill their lines instead of leaving holes at the right.
//
// Works in the chat of global setup's warm-up answer (a reply with measurements); changes nothing.
// `RP_E2E_SHOTS` (optional): review screenshots go there.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { api, charId } from "./api";
import { expect, isPhone, test } from "./fixtures";

const shotsDir = process.env.RP_E2E_SHOTS ?? "";

async function shot(page: Page, name: string, project: string) {
  if (!shotsDir) return;
  mkdirSync(shotsDir, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(shotsDir, `${name}-${project}.png`) });
}

/** The chat of global setup's warm-up (Mara's oldest): a greeting, "Hallo." and an answer with measurements. */
async function warmChat(): Promise<number> {
  const { chats } = await api<{ chats: { id: number }[] }>("GET", `/characters/${await charId("Mara Voss")}/chats`);
  return Math.min(...chats.map((c) => c.id));
}

test("Messwerte: kurz im Blick, ganz im Tooltip, auf schmaler Spalte ganz in eigener Zeile", async ({ page, open }, info) => {
  await open(`/chat/${await warmChat()}`);
  const note = page.locator("article.msg.last .msg-note");
  await expect(note).toBeVisible();
  const full = (await note.getAttribute("title")) ?? "";
  expect(full).toMatch(/Tokens\/s · \d+ Tokens · \d+ neu gelesen, \d+ aus dem Cache/);
  const cut = () => note.evaluate((el) => el.scrollWidth > el.clientWidth + 1);
  if (isPhone(page)) {
    // A phone: all of it, on a line of its own under the actions, wrapped rather than cut.
    await expect(note.locator(".cv-tm-full")).toBeVisible();
    await expect(note.locator(".cv-tm-full")).toHaveText(full);
    await expect(note.locator(".cv-tm-short")).toBeHidden();
    expect(await cut()).toBe(false);
    const [n, actions] = await Promise.all([note.boundingBox(), page.locator("article.msg.last .msg-actions").boundingBox()]);
    expect(n!.y).toBeGreaterThanOrEqual(actions!.y + actions!.height - 1);
  } else {
    await expect(note.locator(".cv-tm-short")).toHaveText(/^\d+ Tokens\/s · \d+ Tokens$/);
    await expect(note.locator(".cv-tm-full")).toBeHidden();
    expect(await cut()).toBe(false);
    // Beside the open panel the row is narrow: the whole line moves under it, nothing is cut.
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.getByRole("button", { name: "Aussehen dieses Chats" }).first().click();
    await expect(page.getByRole("dialog", { name: "Aussehen dieses Chats" })).toBeVisible();
    expect(await cut()).toBe(false);
    await shot(page, "messwerte-panel", info.project.name);
  }
});

test("Hinweiszeile unter dem Feld bleibt einzeilig, auch neben dem Panel", async ({ page, open }, info) => {
  test.skip(isPhone(page), "phones show no key hints");
  await page.setViewportSize({ width: 1280, height: 800 });
  await open(`/chat/${await warmChat()}`);
  const hint = page.locator(".composer .hint");
  await expect(hint).toBeVisible();
  await page.getByRole("button", { name: "Aussehen dieses Chats" }).first().click();
  await expect(page.getByRole("dialog", { name: "Aussehen dieses Chats" })).toBeVisible();
  // One row: every hint shown starts on the first line, the first two (sending, a new line) always.
  const rows = await hint.evaluate((el) => {
    const top = el.getBoundingClientRect().top;
    return [...el.children].map((c) => {
      const r = c.getBoundingClientRect();
      return { first: Math.round(r.top - top) <= 2, shown: r.bottom <= el.getBoundingClientRect().bottom + 3 };
    });
  });
  expect(rows[0]).toEqual({ first: true, shown: true });
  expect(rows[1]).toEqual({ first: true, shown: true });
  for (const r of rows) expect(r.shown ? r.first : true).toBe(true);
  expect((await hint.boundingBox())!.height).toBeLessThanOrEqual(22);
  await shot(page, "hinweise-panel", info.project.name);
});

test("Aussehen öffnet nie leer", async ({ page, open }) => {
  await open(`/chat/${await warmChat()}`);
  // A phone has the button inside the message field, in view while nobody types.
  if (isPhone(page)) await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  const button = isPhone(page) ? page.locator(".cv-look-box") : page.locator(".cv-look-btn");
  await button.click();
  // The very first frame after the click: the panel is there with its title, not a blank pane.
  const first = await page.evaluate(
    () =>
      new Promise<{ opacity: number; title: string }>((done) =>
        requestAnimationFrame(() => {
          const p = document.querySelector(".cv-panel");
          done({ opacity: p ? Number(getComputedStyle(p).opacity) : 0, title: p?.querySelector("h2")?.textContent ?? "" });
        }),
      ),
  );
  expect(first.title).toBe("Aussehen dieses Chats");
  expect(first.opacity).toBeGreaterThanOrEqual(0.5);
  // The ready-made pictures were fetched before the panel needed them (pointer, focus or touch on the button).
  const fetched = await page.evaluate(() => performance.getEntriesByType("resource").filter((e) => e.name.includes("/backdrop-presets/")).length);
  expect(fetched).toBeGreaterThanOrEqual(16);
});

test("Schmale Spalten: Zeilen füllen sich, kein Flattersatz", async ({ page, open }) => {
  await open(`/chat/${await warmChat()}`);
  const wrap = () => page.locator("article.msg .prose").first().evaluate((el) => getComputedStyle(el).textWrap || getComputedStyle(el).getPropertyValue("text-wrap-style"));
  if (isPhone(page)) {
    expect(await wrap()).toBe("wrap");
  } else {
    // A wide window keeps the balanced line ends; beside the open panel the column is narrow.
    expect(await wrap()).toBe("pretty");
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.getByRole("button", { name: "Aussehen dieses Chats" }).first().click();
    await expect(page.getByRole("dialog", { name: "Aussehen dieses Chats" })).toBeVisible();
    expect(await wrap()).toBe("wrap");
  }
});
