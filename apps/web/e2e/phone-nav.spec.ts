// A phone's navigation: the bottom bar with its four places, and "Mehr", the sheet that holds what
// the sidebar holds on a desktop (personas, models, import, search, the connection). On a desktop
// none of it exists on screen. Needs no model answer.

import { mkdirSync } from "node:fs";
import { charId, setLayout, setTheme, token } from "./api";
import { expect, isPhone, snap, test } from "./fixtures";

const SHOTS = process.env.RP_E2E_SHOTS;
const shot = async (page: import("@playwright/test").Page, name: string) => {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: `${SHOTS}/${name}-${test.info().project.name}.png`, animations: "disabled" });
};

const bar = (page: import("@playwright/test").Page) => page.getByRole("navigation", { name: "Bereiche", exact: true });
const sheet = (page: import("@playwright/test").Page) => page.getByRole("dialog", { name: "Mehr" });
/** The sheet has finished sliding: it stands on the bottom edge of the screen. */
const settled = async (page: import("@playwright/test").Page) => {
  await expect
    .poll(async () => {
      const box = await sheet(page).boundingBox();
      return box ? Math.round(box.y + box.height) : 0;
    })
    .toBe(page.viewportSize()!.height);
};

test("Untere Leiste: vier Bereiche, der aktuelle ist markiert", async ({ page, open }) => {
  await setLayout("bisher");
  await open("/");
  if (!isPhone(page)) {
    // A desktop has the sidebar; the bar and its "Mehr" are not on screen.
    await expect(bar(page)).toBeHidden();
    await expect(page.getByRole("button", { name: "Mehr", exact: true })).toBeHidden();
    await expect(page.getByRole("complementary")).toBeVisible();
    return;
  }
  await expect(bar(page).getByRole("link")).toHaveText(["Start", "Charaktere", "Einstellungen"]);
  const more = bar(page).getByRole("button", { name: "Mehr" });
  await expect(more).toBeVisible();
  await expect(bar(page).getByRole("link", { name: "Start" })).toHaveAttribute("aria-current", "page");
  // Every place is large enough for a thumb.
  for (const item of await bar(page).locator("a, button").all()) {
    const box = (await item.boundingBox())!;
    expect(box.height).toBeGreaterThanOrEqual(48);
    expect(box.width).toBeGreaterThanOrEqual(64);
  }
  await bar(page).getByRole("link", { name: "Charaktere" }).click();
  await expect(page).toHaveURL(/#\/chars$/);
  await expect(bar(page).getByRole("link", { name: "Charaktere" })).toHaveAttribute("aria-current", "page");
  await expect(bar(page).getByRole("link", { name: "Start" })).not.toHaveAttribute("aria-current", "page");
  // Inside a character the screen's own header leads and the bar is gone.
  await open(`/char/${await charId("Mara Voss")}/chats`);
  await expect(page.getByRole("heading", { name: "Mara Voss", level: 1 })).toBeVisible();
  await expect(bar(page)).toBeHidden();
});

test("Mehr: Blatt mit Personas, Modellen, Import, Suche und Verbindung", async ({ page, open }) => {
  test.skip(!isPhone(page), "the sheet belongs to the phone's bottom bar");
  await setLayout("bisher");
  await open("/");
  const more = bar(page).getByRole("button", { name: "Mehr" });
  await expect(sheet(page)).toHaveCount(0);
  await more.click();
  await expect(sheet(page)).toBeVisible();
  await expect(more).toHaveAttribute("aria-expanded", "true");
  const rows = sheet(page).getByRole("navigation", { name: "Mehr" });
  await expect(rows.locator("b")).toHaveText(["Personas", "Modelle", "Karten importieren", "Suchen", "Handy verbinden"]);
  // The model's state leads the sheet and opens the tab "Modell".
  await expect(sheet(page).getByRole("status")).toContainText("Modell bereit");
  // The sheet stands on the bottom edge and keeps inside the screen.
  await settled(page);
  const box = (await sheet(page).boundingBox())!;
  const view = page.viewportSize()!;
  expect(box.width).toBe(view.width);
  expect(box.y).toBeGreaterThan(24);
  await snap(page, "mehr");
  await shot(page, "p-mehr");

  // Escape (the back key on a phone) closes it and leaves the page where it was.
  await page.keyboard.press("Escape");
  await expect(sheet(page)).toHaveCount(0);
  await expect(page).toHaveURL(/#\/$/);

  // A row opens its page and the sheet is gone; "Mehr" is then the current place.
  await more.click();
  await rows.getByRole("link", { name: /Personas/ }).click();
  await expect(page).toHaveURL(/#\/personas$/);
  await expect(sheet(page)).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Personas", level: 1 })).toBeVisible();
  await expect(more).toHaveAttribute("data-current", "true");
  await expect(bar(page).locator('[aria-current="page"]')).toHaveCount(0);

  await more.click();
  await rows.getByRole("link", { name: /Modelle/ }).click();
  await expect(page).toHaveURL(/#\/settings\/suggestions$/);
  // The bar follows the page, not the way there: "Vorschläge" is a settings tab.
  await expect(bar(page).getByRole("link", { name: "Einstellungen" })).toHaveAttribute("aria-current", "page");
  await expect(more).not.toHaveAttribute("data-current", "true");

  await more.click();
  await rows.getByRole("link", { name: /Karten importieren/ }).click();
  await expect(page).toHaveURL(/#\/import$/);
  await expect(bar(page).getByRole("link", { name: "Charaktere" })).toHaveAttribute("aria-current", "page");

  await more.click();
  await sheet(page).locator("a.ph-model").click();
  await expect(page).toHaveURL(/#\/settings\/model$/);
  await expect(sheet(page)).toHaveCount(0);

  // "Suchen" hands over to the search window.
  await more.click();
  await rows.getByRole("button", { name: /Suchen/ }).click();
  await expect(sheet(page)).toHaveCount(0);
  await expect(page.getByRole("dialog", { name: "Suchen und Schnellaktionen" })).toBeVisible();
  await page.keyboard.press("Escape");
});

test("Mehr: schließt beim Tippen daneben und beim Herunterziehen", async ({ page, open }) => {
  test.skip(!isPhone(page), "the sheet belongs to the phone's bottom bar");
  await open("/chars");
  const more = bar(page).getByRole("button", { name: "Mehr" });
  await more.click();
  await expect(sheet(page)).toBeVisible();
  // Beside the sheet: its backdrop.
  await page.mouse.click(40, 40);
  await expect(sheet(page)).toHaveCount(0);

  // A short tug lets it spring back; a long one closes it.
  await more.click();
  await settled(page);
  const head = sheet(page).getByRole("heading", { name: "Mehr" });
  const at = (await head.boundingBox())!;
  const x = at.x + 120;
  const y = at.y + at.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + 30, { steps: 6 });
  await page.waitForTimeout(400); // slow, so it is no flick
  await page.mouse.up();
  await expect(sheet(page)).toBeVisible();
  await settled(page);

  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y + 160, { steps: 8 });
  await page.mouse.up();
  await expect(sheet(page)).toHaveCount(0);
  await expect(page).toHaveURL(/#\/chars$/);
});

test("Mehr: die Zurück-Taste schließt das Blatt, die Seite darunter bleibt", async ({ page, open }) => {
  test.skip(!isPhone(page), "the sheet belongs to the phone's bottom bar");
  await open("/");
  await bar(page).getByRole("link", { name: "Charaktere" }).click();
  await expect(page).toHaveURL(/#\/chars$/);
  const more = bar(page).getByRole("button", { name: "Mehr" });
  await more.click();
  await expect(sheet(page)).toBeVisible();
  // The phone's back key: the sheet goes, the page stays.
  await page.goBack();
  await expect(sheet(page)).toHaveCount(0);
  await expect(page).toHaveURL(/#\/chars$/);
  await expect(page.getByRole("heading", { name: "Charaktere", level: 1 })).toBeVisible();

  // A page opened from the sheet takes the sheet's place: one step back is the page before.
  await more.click();
  await sheet(page).getByRole("link", { name: /Personas/ }).click();
  await expect(page).toHaveURL(/#\/personas$/);
  await page.goBack();
  await expect(page).toHaveURL(/#\/chars$/);
  await expect(sheet(page)).toHaveCount(0);

  // Closed by Escape, the sheet leaves no step behind either.
  await more.click();
  await expect(sheet(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(sheet(page)).toHaveCount(0);
  await page.goBack();
  await expect(page).toHaveURL(/#\/$/);
});

test("Mehr in der Handy-App: zurück zur Suche im WLAN", async ({ page, theme }) => {
  test.skip(!isPhone(page), "the sheet belongs to the phone's bottom bar");
  await setTheme(theme);
  await page.goto(`/?token=${token}&shell=android#/`);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await bar(page).getByRole("button", { name: "Mehr" }).click();
  // The contract with the Android app: its own start page, asked to search again.
  await expect(sheet(page).getByRole("link", { name: /Anderen PC verbinden/ })).toHaveAttribute("href", "http://tauri.localhost/?rescan=1");
  await shot(page, "p-mehr-app");
  await page.keyboard.press("Escape");
  await expect(sheet(page)).toHaveCount(0);
});
