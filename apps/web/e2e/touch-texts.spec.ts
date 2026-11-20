// Q1 leftovers: a finger gets no keyboard or mouse hints ("mit Strg K", "Strg + V", "hierher ziehen"),
// a desk keeps them; the tools under the Bühne's head stand clear of it and of the rows below.
// `RP_E2E_SHOTS` (optional): review screenshots go there.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { setLayout } from "./api";
import { expect, isPhone, test } from "./fixtures";

const shots = process.env.RP_E2E_SHOTS;
const KEYS = /Strg|hierher ziehen|zieh es hierher/;

async function shot(page: Page, name: string, theme: string) {
  if (!shots) return;
  mkdirSync(shots, { recursive: true });
  await page.evaluate(() => {
    // Entrances end at once (an endless one, like the lamp's flame, cannot be finished).
    for (const a of document.getAnimations()) if (a.effect?.getTiming().iterations !== Infinity) a.finish();
  });
  await page.screenshot({ path: join(shots, `${name}-${page.viewportSize()!.width}-${theme}.png`) });
}

/** The Bühne's tools: below its head, above the first row, overlapping neither. */
async function toolsClear(page: Page) {
  // On a phone the layout switch sits in the title row at the top; the toolbar under the stage
  // only shows when it has something else to hold (the model strip).
  if (!(await page.locator(".hx-toolbar").isVisible())) {
    expect(isPhone(page)).toBe(true);
    await expect(page.locator(".hx-stage-phone .hx-switch").first()).toBeVisible();
    return;
  }
  const head = (await page.locator("section.stage").boundingBox())!;
  const tools = (await page.locator(".hx-toolbar").boundingBox())!;
  const next = (await page.locator(".stage-rows > section h2.sec").first().boundingBox())!;
  expect(tools.y).toBeGreaterThanOrEqual(head.y + head.height + 8);
  expect(next.y).toBeGreaterThanOrEqual(tools.y + tools.height);
}

async function openStage(page: Page, open: (hash: string) => Promise<void>) {
  await setLayout("buehne");
  await open("#/");
  await expect(page.locator("section.stage")).toBeVisible();
}

/** The settings tab "Allgemein". */
const general = (page: Page) => page.locator(".set").filter({ has: page.getByRole("group", { name: "Start-Layout" }) });

test.afterAll(async () => {
  await setLayout("bisher");
});

test.describe("am PC und am Handy", () => {
  test("Einstellungen nennen Tasten nur am PC; die Bühne-Werkzeuge ragen nirgends hinein", async ({ page, open, theme }) => {
    await open("#/settings");
    await expect(general(page)).toBeVisible();
    if (isPhone(page)) {
      await expect(general(page).getByText(KEYS)).toHaveCount(0);
      await expect(general(page).getByText("zum Vergleichen, auch über die Suche")).toBeVisible();
    } else {
      await expect(general(page).getByText("zum Vergleichen, mit Strg K auch von überall")).toBeVisible();
    }
    await expect(general(page).getByText(/auf dem Handy/)).toHaveCount(0);
    await general(page).getByRole("group", { name: "Start-Layout" }).scrollIntoViewIfNeeded();
    await shot(page, "settings", theme);
    await openStage(page, open);
    await toolsClear(page);
    await shot(page, "stage", theme);
  });
});

test.describe("am Tablet", () => {
  test.use({ viewport: { width: 1280, height: 800 }, hasTouch: true, isMobile: true });
  test.beforeEach(({}, info) => {
    test.skip(info.project.name.startsWith("phone-"), "the tablet tests set their own window size");
  });

  test("keine Tasten- und Maus-Hinweise; die Bühne-Werkzeuge ragen nirgends hinein", async ({ page, open, theme }) => {
    await open("#/settings");
    await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
    await expect(general(page)).toBeVisible();
    await expect(general(page).getByText(KEYS)).toHaveCount(0);
    await expect(general(page).getByText("Wechseln geht unter „Personas“ in der Leiste links.")).toBeVisible();
    await general(page).getByRole("group", { name: "Start-Layout" }).scrollIntoViewIfNeeded();
    await shot(page, "settings-tablet", theme);

    await open("#/personas/new");
    await expect(page.locator(".pic-actions .help")).toContainText("Neues Bild: als Datei oder per Link.");
    await expect(page.getByText(KEYS)).toHaveCount(0);

    await openStage(page, open);
    await toolsClear(page);
    await shot(page, "stage-tablet", theme);
  });
});
