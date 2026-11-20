// The card builder (dictionary `builder`, src/builder/) in English, Spanish and Russian holds no German:
// its three steps on a desktop (1280 px), a phone and a tablet, an entry of the lorebook, and the screen
// after saving. Needs no model answer. With RP_E2E_SHOTS each screen is saved as a picture there.

import type { Locator, Page } from "@playwright/test";
import { api, library } from "./api";
import { expect, test } from "./fixtures";
import { leftoverTests } from "./leftovers";

/** What the user typed: the fields' texts. */
const SKIP = ["textarea"];
const SAVED = "Tove Leftover";

test.afterAll(async () => {
  for (const c of (await library()).characters.filter((x) => x.name === SAVED)) await api("DELETE", `/characters/${c.id}`);
});

async function narrow(page: Page) {
  if ((page.viewportSize()?.width ?? 0) > 1280) await page.setViewportSize({ width: 1280, height: 800 });
}

/** No sideways scrolling, and with RP_E2E_SHOTS a picture of the screen. */
async function shown(page: Page, name: string, region: Locator): Promise<Locator> {
  await expect(region.first()).toBeVisible();
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth), { message: `${name} scrolls sideways` }).toBeLessThanOrEqual(0);
  const dir = process.env.RP_E2E_SHOTS;
  if (dir) {
    const lang = await page.locator("html").getAttribute("lang");
    const form = await page.locator("html").getAttribute("data-form");
    const width = page.viewportSize()?.width ?? 0;
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `${dir}/${lang}-${width}-${form === "tablet" ? "tablet-" : ""}${name}.png`, fullPage: form !== "desktop" });
  }
  return region;
}

const view = (page: Page) => page.locator("div.view").first();
const stepButton = (page: Page, n: number) => page.locator(".cb-steps button").nth(n);

async function plan(page: Page) {
  await narrow(page);
  await expect(page.locator(".cb-outline")).toBeVisible();
  return shown(page, "builder-plan", view(page));
}

async function build(page: Page) {
  await narrow(page);
  await stepButton(page, 1).click();
  // One lorebook entry, so its texts are read too.
  await page.locator('[data-spot="builder-lore"] .btn-row .btn').click();
  await expect(page.locator(".cb-lore")).toBeVisible();
  return shown(page, "builder-build", view(page));
}

async function preview(page: Page) {
  await narrow(page);
  await stepButton(page, 2).click();
  await expect(page.locator(".cb-pv.full")).toBeVisible();
  return shown(page, "builder-preview", view(page));
}

async function saved(page: Page) {
  await narrow(page);
  await stepButton(page, 1).click();
  await page.locator("#cb-name").fill(SAVED);
  // Saving is the bar's last button (with a name alone "Weiter" still leads).
  await page.locator(".cb-bar .btn").last().click();
  await expect(page.locator(".cb-done")).toBeVisible();
  return shown(page, "builder-saved", view(page));
}

leftoverTests("builder", [
  { name: "Ersteller: Plan", form: "desktop", hash: "/chars/new", region: plan, skip: SKIP },
  { name: "Ersteller: Bauen", form: "desktop", hash: "/chars/new", region: build, skip: SKIP },
  { name: "Ersteller: Vorschau", form: "desktop", hash: "/chars/new", region: preview, skip: SKIP },
  { name: "Ersteller: Gespeichert", form: "desktop", hash: "/chars/new", region: saved, skip: SKIP },
  { name: "Ersteller: Plan", form: "phone", hash: "/chars/new", region: plan, skip: SKIP },
  { name: "Ersteller: Bauen", form: "phone", hash: "/chars/new", region: build, skip: SKIP },
  { name: "Ersteller: Vorschau", form: "phone", hash: "/chars/new", region: preview, skip: SKIP },
  { name: "Ersteller: Plan", form: "tablet", hash: "/chars/new", region: plan, skip: SKIP },
  { name: "Ersteller: Bauen", form: "tablet", hash: "/chars/new", region: build, skip: SKIP },
]);
