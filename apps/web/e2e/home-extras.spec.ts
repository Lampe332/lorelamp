// Start screen, what all five layouts share: the layout switch on the screen itself, a new chat and
// the star on a figure, new imports, the first steps while nothing is there yet, and the model's
// action. Everything a test changes (layout, stars, characters, chats, the loaded model) is put back.

import { mkdirSync } from "node:fs";
import { api, deleteChat, fixture, library, setLayout } from "./api";
import { expect, isPhone, test } from "./fixtures";
import type { Page } from "@playwright/test";

const LAYOUTS = ["bisher", "dashboard", "bento", "buehne", "kommando"] as const;
const SHOTS = process.env.RP_E2E_SHOTS;

test.afterAll(async () => {
  await setLayout("bisher");
});

const shown = (page: Page, name: string | RegExp) => page.getByRole("button", { name }).filter({ visible: true }).first();
const layoutSwitch = (page: Page) => page.getByRole("button", { name: /^Start-Layout wechseln/ });

async function shot(page: Page, name: string) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: `${SHOTS}/${name}.png`, animations: "disabled" });
}

for (const layout of LAYOUTS) {
  test(`${layout}: Layout-Menü, neuer Chat mit einer Figur, Geplantes`, async ({ page, open }) => {
    await setLayout(layout);
    await open("/");
    // The switch names the layout on screen and offers all five.
    await layoutSwitch(page).click();
    const menu = page.getByRole("menu", { name: "Start-Layout" });
    await expect(menu.getByRole("menuitemradio")).toHaveCount(5);
    await expect(menu.getByRole("menuitemradio", { checked: true })).toHaveCount(1);
    await expect(menu.getByRole("menuitemradio", { checked: true })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    await expect(layoutSwitch(page)).toBeFocused();
    // What is planned is one line, not tiles; the model library is built and no longer on it.
    const soon = page.getByRole("list", { name: "Bald in der App" });
    await expect(soon.getByRole("listitem")).toHaveCount(4);
    await expect(soon).not.toContainText("Modell");
    // A fresh chat with a figure is one press away.
    const before = (await library()).characters.find((c) => c.name === "Mara Voss")!.chats;
    await shown(page, "Neuer Chat mit Mara Voss").click();
    await expect(page).toHaveURL(/#\/chat\/\d+$/);
    const chatId = Number(/#\/chat\/(\d+)$/.exec(page.url())![1]);
    expect((await library()).characters.find((c) => c.name === "Mara Voss")!.chats).toBe(before + 1);
    await deleteChat(chatId);
  });
}

test("Layout auf der Startseite wechseln, mit Maus und Tastatur", async ({ page, open }) => {
  await setLayout("bisher");
  await open("/");
  await layoutSwitch(page).click();
  await page.getByRole("menuitemradio", { name: /^Bühne/ }).click();
  await expect(page.getByRole("region", { name: "Zuletzt" }).getByRole("heading", { level: 1 })).toBeVisible();
  await expect.poll(async () => (await library()).home_layout).toBe("buehne");
  await expect(layoutSwitch(page)).toHaveAccessibleName(/Bühne/);
  // Back with the keyboard: the menu opens on the current entry, "Klassisch" (id bisher) is the first.
  await layoutSwitch(page).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("menuitemradio", { name: /^Bühne/ })).toBeFocused();
  await page.keyboard.press("Home");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "Start", level: 1 })).toBeVisible();
  await expect.poll(async () => (await library()).home_layout).toBe("bisher");
});

test("Bühne auf dem Handy: Titel und Layout-Schalter oben, wie in den anderen Layouts", async ({ page, open }) => {
  test.skip(!isPhone(page), "the phone's head of the stage");
  await setLayout("buehne");
  await open("/");
  const stage = page.getByRole("region", { name: "Zuletzt" });
  await expect(stage).toBeVisible();
  await expect(page.locator(".hx-stage-title")).toHaveText("Start");
  const sw = layoutSwitch(page);
  await expect(sw).toHaveCount(1); // the one under the stage is the desktop's
  expect((await sw.boundingBox())!.y).toBeLessThan((await stage.boundingBox())!.y);
  await expect(sw).toHaveAccessibleName(/Bühne/);
});

test("Navigation: nichts Geplantes mit „Bald“, die Startseite nennt es einmal", async ({ page, open }) => {
  await setLayout("bisher");
  await open("/");
  if (!isPhone(page)) {
    const side = page.locator("aside.side");
    await expect(side).toBeVisible();
    await expect(side).not.toContainText(/Bald|Gruppen-Chat|Lorebooks|Statistik|Gedächtnis/);
    // What "Mehr" held besides them is in the list itself now.
    await expect(side.getByRole("link", { name: "Handy verbinden" })).toBeVisible();
  }
  await expect(page.getByRole("list", { name: "Bald in der App" })).toBeAttached();
});

test("Favorit: der Stern gilt wie in der Bibliothek und stellt die Figur nach vorn", async ({ page, open }) => {
  const ids = (await library()).characters.map((c) => c.id);
  await api("POST", "/characters/bulk", { ids, action: "unfavorite" });
  try {
    await setLayout("dashboard");
    await open("/");
    const figures = page.getByRole("region", { name: /^Deine Figuren/ });
    const star = figures.getByRole("button", { name: "Kapitän Ilse Brandt: Favorit" });
    await expect(star).toHaveAttribute("aria-pressed", "false");
    await star.click({ force: true }); // it shows on hover where a pointer hovers
    await expect(star).toHaveAttribute("aria-pressed", "true");
    await expect
      .poll(async () => (await api<{ characters: { name: string; favorite: boolean }[] }>("GET", "/library")).characters.find((c) => c.name === "Kapitän Ilse Brandt")?.favorite)
      .toBe(true);
    await expect(figures.getByRole("link", { name: /Mara Voss|Moos|Kapitän/ }).first()).toContainText("Kapitän Ilse Brandt");
    await star.click({ force: true });
    await expect(star).toHaveAttribute("aria-pressed", "false");
  } finally {
    await api("POST", "/characters/bulk", { ids, action: "unfavorite" });
  }
});

test("Neu importiert: eine ungespielte Figur steht in jedem Layout bereit", async ({ page, open }) => {
  const card = await api<{ id: number }>("POST", "/characters/import", fixture("sillytavern/greta_holm.json"));
  try {
    for (const layout of LAYOUTS) {
      await setLayout(layout);
      await open("/");
      const fresh = page.getByRole("region", { name: /^Neu importiert/ });
      await expect(fresh.getByRole("link", { name: /Greta Holm/ })).toBeVisible();
      await expect(fresh.getByRole("button", { name: "Neuer Chat mit Greta Holm" })).toContainText("Ersten Chat starten");
      if (isPhone(page)) {
        // A phone keeps the name whole: the new chat is one round plus of 48 px.
        const plus = (await fresh.getByRole("button", { name: "Neuer Chat mit Greta Holm" }).boundingBox())!;
        expect(Math.round(plus.width)).toBe(48);
        expect(Math.round(plus.height)).toBe(48);
        const name = fresh.locator(".hx-line-text b").filter({ hasText: "Greta Holm" });
        expect(await name.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
      }
      await shot(page, `neu-importiert-${layout}-${test.info().project.name}`);
    }
    await page.getByRole("region", { name: /^Neu importiert/ }).getByRole("button", { name: "Neuer Chat mit Greta Holm" }).click();
    await expect(page).toHaveURL(/#\/chat\/\d+$/);
    // Played now: no longer "new".
    await page.goBack();
    await expect(page.getByRole("heading", { name: "Was möchtest du tun?" })).toBeVisible();
    await expect(page.getByRole("region", { name: /^Neu importiert/ })).toHaveCount(0);
  } finally {
    await api("DELETE", `/characters/${card.id}`);
  }
});

test("Leerer Zustand: erste Schritte in jedem Layout, mit Weg zum Import", async ({ page, open }) => {
  const ids = (await library()).characters.map((c) => c.id);
  await api("POST", "/characters/bulk", { ids, action: "delete" });
  try {
    for (const layout of LAYOUTS) {
      await setLayout(layout);
      await open("/");
      // The four steps of the welcome tour; the title counts the open ones.
      const done = 1 + (await named()); // the test server's model runs
      const first = page.getByRole("region", { name: firstTitle(done) });
      await expect(first).toBeVisible();
      await expect(first.getByRole("listitem")).toHaveCount(4);
      await expect(first).toContainText(`${done} von 4 erledigt`);
      await expect(first.getByRole("button", { name: "Aus SillyTavern übernehmen" })).toBeVisible();
      await expect(layoutSwitch(page)).toBeVisible();
      await shot(page, `leer-${layout}-${test.info().project.name}`);
      await first.getByRole("button", { name: "Karte importieren" }).click();
      await expect(page).toHaveURL(/#\/import$/);
    }
  } finally {
    await api("POST", "/characters/bulk", { ids, action: "restore" });
  }
  expect((await library()).characters.length).toBe(ids.length);
});

/** Whether the user has a name of their own ("User" is the one a new installation starts with). */
const named = async () => Number((await library()).user_name.trim() !== "User");
const firstTitle = (done: number) => (done === 0 ? "Hier beginnt deine erste Geschichte" : done === 3 ? "Noch ein Schritt bis zur ersten Geschichte" : `Noch ${4 - done} Schritte bis zur ersten Geschichte`);

test("Erste Schritte: der Name ist ein Schritt wie in der Einführung, der Titel zählt die offenen", async ({ page, open }) => {
  const ids = (await library()).characters.map((c) => c.id);
  const before = (await library()).user_name;
  await api("POST", "/characters/bulk", { ids, action: "delete" });
  try {
    await api("PUT", "/settings", { user_name: "User" });
    await setLayout("bisher");
    await open("/");
    let first = page.getByRole("region", { name: "Noch 3 Schritte bis zur ersten Geschichte" });
    await expect(first).toContainText("1 von 4 erledigt");
    const step = first.getByRole("listitem").nth(2);
    await expect(step).toContainText("Deinen Namen festlegen");
    const lib = await library();
    await expect(step.getByRole("link", { name: "Namen festlegen" })).toHaveAttribute("href", `#/personas/${lib.active_persona_id}`);
    // A name set elsewhere (here: the tour's way, the server's setting) arrives with the library.
    await api("PUT", "/settings", { user_name: "Q9 Leserin" });
    first = page.getByRole("region", { name: "Noch 2 Schritte bis zur ersten Geschichte" });
    await expect(first).toContainText("2 von 4 erledigt");
    await expect(first.getByRole("listitem").nth(2)).toContainText("Die Figuren nennen dich Q9 Leserin.");
    await expect(first.getByRole("link", { name: "Namen festlegen" })).toHaveCount(0);
  } finally {
    await api("PUT", "/settings", { user_name: before });
    await api("POST", "/characters/bulk", { ids, action: "restore" });
  }
});

test("Figuren, aber noch kein Chat: der vierte Schritt startet ihn", async ({ page, open }) => {
  const chats: number[] = [];
  for (const c of (await library()).characters) {
    const list = await api<{ chats: { id: number }[] }>("GET", `/characters/${c.id}/chats`);
    chats.push(...list.chats.map((x) => x.id));
  }
  for (const id of chats) await deleteChat(id);
  let made = 0;
  try {
    for (const layout of LAYOUTS) {
      await setLayout(layout);
      await open("/");
      const done = 2 + (await named());
      const first = page.getByRole("region", { name: firstTitle(done) });
      await expect(first).toContainText(`${done} von 4 erledigt`);
      await expect(first.getByRole("button", { name: /^Neuer Chat mit / })).toHaveCount(3);
      await shot(page, `ohne-chat-${layout}-${test.info().project.name}`);
    }
    await page.getByRole("region", { name: /bis zur ersten Geschichte$/ }).getByRole("button", { name: "Neuer Chat mit Moos" }).click();
    await expect(page).toHaveURL(/#\/chat\/\d+$/);
    made = Number(/#\/chat\/(\d+)$/.exec(page.url())![1]);
  } finally {
    if (made) await deleteChat(made);
    for (const id of chats) await api("POST", `/chats/${id}/restore`);
  }
});

test("Modell entladen: die Startseite sagt es und lädt es auf einen Klick", async ({ page, open }) => {
  const engine = await api<{ managed: boolean }>("GET", "/engine");
  test.skip(!engine.managed, "needs a test server that runs the model itself (--manage-engine)");
  test.setTimeout(180_000);
  await api("POST", "/engine/unload");
  try {
    for (const layout of ["bisher", "bento"] as const) {
      await setLayout(layout);
      await open("/");
      // The strip says it in its title, the model tile in its status and one sentence.
      await expect(page.getByText(layout === "bisher" ? "Modell entladen" : "Es lädt von selbst, sobald du etwas sendest.").filter({ visible: true }).first()).toBeVisible();
      await expect(shown(page, "Jetzt laden")).toBeVisible();
    }
    await shot(page, `modell-entladen-bento-${test.info().project.name}`);
    await shown(page, "Jetzt laden").click();
    await expect.poll(async () => (await api<{ status: string }>("GET", "/engine")).status, { timeout: 120_000 }).toBe("ready");
    await expect(page.getByRole("button", { name: "Jetzt laden" })).toHaveCount(0);
  } finally {
    if ((await api<{ unloaded: boolean }>("GET", "/engine")).unloaded) await api("POST", "/engine/load");
    await expect.poll(async () => (await api<{ status: string }>("GET", "/engine")).status, { timeout: 120_000 }).toBe("ready");
  }
});

// Review pictures for the owner (only with RP_E2E_SHOTS): every layout at the project's size, on the
// desktop also at 2560 and 768 (Graphit) or the whole page (Papier), on the phone the whole page too.
test("Bilder aller Layouts", async ({ page, open }) => {
  test.skip(!SHOTS, "set RP_E2E_SHOTS to save review pictures");
  const [kind, theme] = test.info().project.name.split("-");
  const base = page.viewportSize()!;
  for (const layout of LAYOUTS) {
    await setLayout(layout);
    await open("/");
    await shot(page, `${layout}-${base.width}-${theme}`);
    if (isPhone(page)) {
      await page.setViewportSize({ width: 375, height: 2400 });
      await shot(page, `${layout}-375-${theme}-ganz`);
    } else if (kind === "desktop" && theme === "graphit") {
      await page.setViewportSize({ width: 2560, height: 1440 });
      await shot(page, `${layout}-2560-${theme}`);
      await page.setViewportSize({ width: 768, height: 1024 });
      await shot(page, `${layout}-768-${theme}`);
    } else {
      await page.setViewportSize({ width: 1920, height: 2200 });
      await shot(page, `${layout}-1920-${theme}-ganz`);
    }
    await page.setViewportSize(base);
  }
});
