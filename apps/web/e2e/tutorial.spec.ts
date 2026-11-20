// Q9: the welcome tour of the first start (src/tutorial/). Runs in the main suite and with
// e2e/first-run.config.ts against an empty data folder: there the tour must come by itself (the test
// "Neue Daten" skips itself on a server that has characters). Everything else starts the tour the way
// a user does again (Settings → Allgemein, Ctrl+K) or puts its step on the server, and leaves the
// server as it found it: step, language and the user's name.
// With `RP_E2E_SHOTS` every step is saved as a picture there, in German and English, at 1280 and 375 px.

import { mkdirSync } from "node:fs";
import type { Page } from "@playwright/test";
import { api, library } from "./api";
import { expect, isPhone, test } from "./fixtures";

const SHOTS = process.env.RP_E2E_SHOTS;
type Step = "welcome" | "model" | "figure" | "persona" | "chat" | "done";

const tourStep = async () => (await api<{ step: Step }>("GET", "/tutorial")).step;
const putStep = (step: Step) => api("PUT", "/tutorial", { step });
const tour = (page: Page) => page.getByRole("dialog");
const heading = (page: Page) => tour(page).getByRole("heading", { level: 2 });

let before: Step = "done";
let name = "";
test.beforeAll(async () => {
  before = await tourStep();
  name = (await library()).user_name;
});
test.afterAll(async () => {
  await api("PUT", "/settings", { language: "de", user_name: name });
  await putStep(before);
});

test("Neue Daten: die Einführung kommt von selbst, mit Sprache, Logo und GitHub", async ({ page, open }) => {
  const lib = await library();
  test.skip(lib.characters.length > 0, "needs an empty data folder (e2e/first-run.config.ts)");
  await expect.poll(tourStep).toBe("welcome");
  await open("/");
  await expect(tour(page)).toBeVisible();
  await expect(heading(page)).toHaveText("Lorelamp");
  await expect(tour(page).locator("svg.tu-mark")).toBeVisible();
  await expect(tour(page).getByRole("radio")).toHaveCount(4);
  await expect(tour(page).getByRole("radio", { name: "Deutsch" })).toBeChecked();
  await expect(tour(page)).toContainText("Gemacht von Lampe");
  await expect(tour(page).getByRole("link", { name: "Quellcode auf GitHub" })).toHaveAttribute("href", "https://github.com/Lampe332/lorelamp");
  // Nothing behind it takes a click or the focus while it is open.
  await expect(page.locator(".tu-start")).toBeFocused();
});

test("Sprache umschalten wechselt sofort die Texte", async ({ page, open }) => {
  await putStep("welcome");
  await open("/");
  await expect(tour(page).getByRole("button", { name: "Los geht's" })).toBeVisible();
  await tour(page).getByRole("radio", { name: "English" }).check();
  await expect(tour(page).getByRole("button", { name: "Let's go" })).toBeVisible();
  await expect(tour(page)).toContainText("Tell stories with characters who answer back.");
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  // The choice lands on the server (every device follows) and the focus stays on it.
  await expect.poll(async () => (await library()).language).toBe("en");
  await expect(tour(page).getByRole("radio", { name: "English" })).toBeFocused();
  await tour(page).getByRole("radio", { name: "Русский" }).check();
  await expect(tour(page).getByRole("button", { name: "Начать" })).toBeVisible();
  await tour(page).getByRole("radio", { name: "Deutsch" }).check();
  await expect(tour(page).getByRole("button", { name: "Los geht's" })).toBeVisible();
  await expect.poll(tourStep).toBe("welcome");
});

test("Durchklicken bis zum Ende: Name gesetzt, danach kommt sie nach Neuladen nicht wieder", async ({ page, open }) => {
  await putStep("welcome");
  await open("/");
  await tour(page).getByRole("button", { name: "Los geht's" }).click();
  await expect(heading(page)).toHaveText("Ein Modell einrichten");
  await expect(heading(page)).toBeFocused();
  await expect(tour(page)).toContainText("Schritt 1 von 4");
  await expect(tour(page).locator('.tu-dots [aria-current="step"]')).toHaveCount(1);
  // Done or not: one press goes on (Weiter once the model is there, else Überspringen).
  await tour(page).getByRole("button", { name: /^(Weiter|Überspringen)$/ }).click();
  await expect(heading(page)).toHaveText("Eine Figur holen");
  await expect(tour(page).getByRole("link", { name: "Karte importieren" })).toHaveAttribute("href", "#/import");
  await expect(tour(page).getByRole("link", { name: "Neue Figur bauen" })).toHaveAttribute("href", "#/chars/new");
  await tour(page).getByRole("button", { name: /^(Weiter|Überspringen)$/ }).click();
  await expect(heading(page)).toHaveText("Wer bist du in der Geschichte?");
  const field = tour(page).getByRole("textbox", { name: "Dein Name" });
  // The name a new installation starts with ("User") is not the user's: the field starts empty then.
  await expect(field).toHaveValue(name === "User" ? "" : name);
  await expect(field).toHaveAttribute("placeholder", "zum Beispiel Alex");
  await field.fill("Q9 Leserin");
  await field.press("Enter"); // the form's own way on
  await expect(heading(page)).toHaveText("Dein erster Chat");
  await expect.poll(async () => (await library()).user_name).toBe("Q9 Leserin");
  await api("PUT", "/settings", { user_name: name });
  await tour(page).getByRole("button", { name: "Zurück" }).click();
  await expect(heading(page)).toHaveText("Wer bist du in der Geschichte?");
  await tour(page).getByRole("button", { name: "Überspringen" }).click();
  await tour(page).getByRole("button", { name: "Fertig" }).click();
  await expect(tour(page)).toHaveCount(0);
  await expect.poll(tourStep).toBe("done");
  await page.reload();
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  await expect(page.locator(".tu-dock, dialog.tu")).toHaveCount(0);
});

test("Modellschritt: nie ein leeres Feld, ein Satz und ein Knopf in einfachen Worten", async ({ page, open }) => {
  await putStep("model");
  await open("/");
  await expect(heading(page)).toHaveText("Ein Modell einrichten");
  // From its first frame the step holds its panel (the real one or its placeholder), never an empty box.
  await expect(tour(page).locator(".tu-panel, .tu-done").first()).toBeVisible();
  expect(await tour(page).locator(".tu-panel:empty").count()).toBe(0);
  await expect(page.locator('dialog.tu [aria-busy="true"]')).toHaveCount(0);
  const panel = tour(page).locator(".tu-panel");
  if ((await panel.count()) > 0) {
    await expect(panel.locator("p")).toHaveCount(1);
    await expect(panel).not.toContainText(/Engine|Build/);
    await expect(panel.locator("a, button")).toHaveCount(1);
  }
  await putStep("done");
});

test("Später beendet sie, ein Schritt bleibt beim Neuladen erhalten", async ({ page, open }) => {
  await putStep("welcome");
  await open("/");
  await tour(page).getByRole("button", { name: "Los geht's" }).click();
  await tour(page).getByRole("button", { name: /^(Weiter|Überspringen)$/ }).click();
  await expect(heading(page)).toHaveText("Eine Figur holen");
  await expect.poll(tourStep).toBe("figure");
  await page.reload();
  await expect(heading(page)).toHaveText("Eine Figur holen");
  await expect(tour(page)).toContainText("Schritt 2 von 4");
  await tour(page).getByRole("button", { name: "Später" }).click();
  await expect(tour(page)).toHaveCount(0);
  await expect.poll(tourStep).toBe("done");
  await page.reload();
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  await expect(page.locator(".tu-dock, dialog.tu")).toHaveCount(0);
});

test("Ein Weg in die App legt sie an den Rand, Escape auch; ein Tipp holt sie zurück", async ({ page, open }) => {
  await putStep("figure");
  await open("/");
  await tour(page).getByRole("link", { name: "Karte importieren" }).click();
  await expect(page).toHaveURL(/#\/import$/);
  await expect(tour(page)).toHaveCount(0);
  const dock = page.getByRole("button", { name: /Weiter mit der Einführung/ });
  await expect(dock).toBeVisible();
  await expect(dock).toContainText("Schritt 2 von 4");
  await dock.click();
  await expect(heading(page)).toHaveText("Eine Figur holen");
  // The keyboard: Tab stays inside the window, Escape puts it aside (not away).
  for (let i = 0; i < 8; i++) await page.keyboard.press("Tab");
  expect(await page.evaluate(() => !!document.activeElement?.closest("dialog.tu"))).toBe(true);
  await page.keyboard.press("Escape");
  await expect(tour(page)).toHaveCount(0);
  await expect(dock).toBeVisible();
  await expect.poll(tourStep).toBe("figure");
  await dock.press("Enter");
  await expect(heading(page)).toHaveText("Eine Figur holen");
  await putStep("done");
});

test("Erneut starten in Einstellungen → Allgemein (und am PC mit Strg+K)", async ({ page, open }) => {
  await putStep("done");
  await open("/settings");
  const row = page.locator('[data-spot="tutorial"]');
  await expect(row).toContainText("Einführung");
  await row.getByRole("button", { name: "Neu starten" }).click();
  await expect(heading(page)).toHaveText("Lorelamp");
  await expect.poll(tourStep).toBe("welcome");
  await tour(page).getByRole("button", { name: "Später" }).click();
  await expect(tour(page)).toHaveCount(0);
  if (!isPhone(page)) {
    await page.keyboard.press("Control+k");
    await page.keyboard.type("Einführung neu");
    await page.getByRole("option", { name: /Einführung neu starten/ }).first().click();
    await expect(heading(page)).toHaveText("Lorelamp");
    await tour(page).getByRole("button", { name: "Später" }).click();
  }
  await expect.poll(tourStep).toBe("done");
});

test("Erster Chat aus der Einführung", async ({ page, open }) => {
  const lib = await library();
  test.skip(lib.characters.length === 0, "needs a character");
  await putStep("chat");
  await open("/");
  await expect(heading(page)).toHaveText("Dein erster Chat");
  const first = tour(page).locator(".tu-fig").first();
  await expect(first).toBeVisible();
  await first.click();
  await expect(page).toHaveURL(/#\/chat\/\d+$/);
  await expect(tour(page)).toHaveCount(0);
  await expect.poll(tourStep).toBe("done");
  await api("DELETE", `/chats/${/#\/chat\/(\d+)$/.exec(page.url())![1]}`);
});

test.describe("Tablet", () => {
  test.use({ viewport: { width: 1280, height: 800 }, hasTouch: true, isMobile: true });
  test("Tablet: Fenster in der Mitte, Knöpfe groß genug", async ({ page, open }, info) => {
    test.skip(info.project.name.startsWith("phone-"), "a tablet window");
    await putStep("persona");
    await open("/");
    await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
    const sheet = page.locator(".tu-sheet");
    const box = (await sheet.boundingBox())!;
    expect(Math.abs(box.x + box.width / 2 - 640)).toBeLessThan(2);
    expect(box.width).toBeLessThanOrEqual(600);
    for (const b of await tour(page).getByRole("button").all()) expect((await b.boundingBox())!.height).toBeGreaterThanOrEqual(48);
    await tour(page).getByRole("button", { name: "Später" }).click();
    await expect(tour(page)).toHaveCount(0);
  });
});

test("Handy und Desktop: nichts ragt seitlich heraus", async ({ page, open }) => {
  await putStep("welcome");
  await open("/");
  for (const step of ["welcome", "model", "figure", "persona", "chat"] as const) {
    await putStep(step);
    await page.reload();
    await expect(tour(page)).toBeVisible();
    const sheet = (await page.locator(".tu-sheet").boundingBox())!;
    const width = page.viewportSize()!.width;
    expect(sheet.x).toBeGreaterThanOrEqual(0);
    expect(sheet.x + sheet.width).toBeLessThanOrEqual(width);
    expect(await page.locator(".tu-body").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    if (isPhone(page)) expect(sheet.width).toBe(width); // the whole screen
  }
  await putStep("done");
});

test("Bilder aller Schritte (de, en; 1280 und 375 px)", async ({ page, open }, info) => {
  test.skip(!SHOTS, "RP_E2E_SHOTS not set");
  const project = info.project.name;
  if (!isPhone(page)) await page.setViewportSize({ width: 1280, height: 800 });
  mkdirSync(SHOTS!, { recursive: true });
  for (const language of ["de", "en"]) {
    await api("PUT", "/settings", { language });
    await putStep("welcome");
    await open("/");
    await expect(page.locator("html")).toHaveAttribute("lang", language);
    for (const [i, step] of (["welcome", "model", "figure", "persona", "chat"] as const).entries()) {
      await putStep(step);
      await page.reload();
      await expect(tour(page)).toBeVisible();
      await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
      await page.evaluate(() => document.fonts.ready);
      await page.waitForTimeout(450); // the step's slide-in
      await page.screenshot({ path: `${SHOTS}/${i + 1}-${step}-${language}-${project}.png` });
    }
    await putStep("figure");
    await page.reload();
    await tour(page).getByRole("link", { name: language === "de" ? "Karte importieren" : "Import card" }).click();
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${SHOTS}/6-dock-${language}-${project}.png` });
    await page.locator(".tu-dock").click(); // put aside stays so on this device, also after a reload
  }
  await api("PUT", "/settings", { language: "de" });
  await putStep("done");
  await open("/settings");
  await page.locator('[data-spot="tutorial"]').scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${SHOTS}/7-settings-de-${project}.png` });
});
