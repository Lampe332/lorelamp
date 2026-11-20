// Settings tab "Modell": "Modell wechseln" and "Beste Einstellung finden" open and close on their own,
// so both panels can be open at once, and with both open the end of the page can still be scrolled to:
// the last card ("Alles genau einstellen") is shown whole, with the page's end space below it.
//
// With RP_E2E_SHOTS set to a folder, the page end with both panels open is saved there for a look.

import { join } from "node:path";
import { expect, test } from "./fixtures";
import type { Page } from "@playwright/test";

const shots = process.env.RP_E2E_SHOTS;
async function shot(page: Page, name: string) {
  if (!shots) return;
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(shots, `${name}-${test.info().project.name}.png`) });
}

test("both model panels open at once and the page end stays reachable", async ({ page, open }) => {
  await open("/settings/model");
  const hero = page.locator("section.mx-hero");
  const switchBtn = hero.getByRole("button", { name: "Modell wechseln" });
  const benchBtn = hero.getByRole("button", { name: "Beste Einstellung finden" });
  const switchPanel = page.getByRole("region", { name: "Meine Modelle" });
  const benchPanel = page.getByRole("region", { name: "Beste Einstellung finden" });

  await switchBtn.click();
  await expect(switchBtn).toHaveAttribute("aria-expanded", "true");
  await benchBtn.click();
  // Opening the second panel leaves the first one open.
  await expect(benchBtn).toHaveAttribute("aria-expanded", "true");
  await expect(switchBtn).toHaveAttribute("aria-expanded", "true");
  await expect(switchPanel).toBeVisible();
  await expect(benchPanel).toBeVisible();

  // Scroll the settings view to its end (again, until the smooth scroll of the opened panel has settled).
  const view = page.locator(".view", { has: page.locator(".mx-simple") });
  const last = page.getByRole("link", { name: /Alles genau einstellen/ });
  await expect
    .poll(
      () =>
        view.evaluate((el) => {
          el.scrollTop = el.scrollHeight;
          return el.scrollHeight - el.clientHeight - el.scrollTop;
        }),
      { intervals: [300, 300, 500] },
    )
    .toBeLessThanOrEqual(1);
  await expect(page.getByRole("link", { name: /Neue Modelle finden/ })).toBeInViewport({ ratio: 1 });
  await expect(last).toBeInViewport({ ratio: 1 });
  // The last card does not end flush with the window edge: the page keeps its end space below it.
  const gap = await last.evaluate((el) => {
    const scroller = el.closest(".view")!;
    return scroller.getBoundingClientRect().bottom - el.getBoundingClientRect().bottom;
  });
  expect(gap).toBeGreaterThanOrEqual(32);
  await shot(page, "model-panels-end");

  // Closing one panel leaves the other one open.
  await switchBtn.scrollIntoViewIfNeeded();
  await switchBtn.click();
  await expect(switchBtn).toHaveAttribute("aria-expanded", "false");
  await expect(switchPanel).toHaveCount(0);
  await expect(benchBtn).toHaveAttribute("aria-expanded", "true");
  await expect(benchPanel).toBeVisible();
  await benchBtn.click();
  await expect(benchPanel).toHaveCount(0);
});
