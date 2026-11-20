// The characters tab: the search that is always there (name, short text, tags, with typos), the tag
// filter over the card's and the own tags, the own short text under the card (set in the character
// settings, "Automatisch" goes back), and the three layouts (kept on the server and after a reload).

import type { Page } from "@playwright/test";
import { api, charId, library } from "./api";
import { expect, test } from "./fixtures";

const names = (page: Page) => page.locator(".grid-chars .cc .body b");
const palette = (page: Page) => page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
const layoutOf = async () => (await api<{ library_layout: string }>("GET", "/library")).library_layout;

async function reset() {
  await api("PUT", "/settings", { library_layout: "poster" });
  const ids = (await library()).characters.map((c) => c.id);
  await api("POST", "/characters/bulk", { ids, action: "unfavorite" });
  for (const id of ids) await api("PUT", `/characters/${id}/tags`, { tags: [] });
}

test.afterEach(reset);

test("Charaktere: Suche mit Tippfehler, Kurztext und Tags", async ({ page, open }) => {
  await open("/chars");
  const search = page.getByRole("textbox", { name: "Charaktere durchsuchen" });
  await expect(search).toBeVisible(); // also with only three cards

  // A typo in the name.
  await search.fill("Brnadt");
  await expect(names(page)).toHaveText(["Kapitän Ilse Brandt"]);
  await expect(page.getByText("1 von 3")).toBeVisible();
  // A word of the short text, and a card tag.
  await search.fill("leuchtturm");
  await expect(names(page)).toHaveText(["Mara Voss"]);
  await search.fill("Nordsee");
  await expect(names(page)).toHaveText(["Mara Voss"]);
  await search.fill("gemutlich");
  await expect(names(page)).toHaveText(["Moos"]);
  await search.fill("Quallenfisch");
  await expect(page.getByText("Keine Karte passt zu „Quallenfisch“.")).toBeVisible();
  // Escape empties the field.
  await search.press("Escape");
  await expect(search).toHaveValue("");
  await expect(names(page)).toHaveCount(3);
});

test("Charaktere: Filter nach Karten-Tags und eigenen Tags, kombinierbar", async ({ page, open }) => {
  // An own tag in another spelling of a card tag counts as the same tag.
  await api("PUT", `/characters/${await charId("Moos")}/tags`, { tags: ["abenteuer"] });
  await open("/chars");
  const filter = page.getByRole("group", { name: "Nach Tag filtern" });
  const chip = (name: string) => filter.getByRole("button", { name: new RegExp(`^${name}\\s*\\d+$`, "i") });

  // Card tags only: "Fantasy" is Moos's.
  await chip("Fantasy").click();
  await expect(chip("Fantasy")).toHaveAttribute("aria-pressed", "true");
  await expect(names(page)).toHaveText(["Moos"]);
  // One chip for "Abenteuer" and "abenteuer", counted three times; both tags together: still Moos.
  await expect(chip("Abenteuer")).toHaveCount(1);
  await expect(chip("Abenteuer")).toContainText("3");
  await chip("Abenteuer").click();
  await expect(names(page)).toHaveText(["Moos"]);
  // Without "Fantasy": all three; with "Nordsee" as well: only Mara.
  await chip("Fantasy").click();
  await expect(names(page)).toHaveCount(3);
  await chip("Nordsee").click();
  await expect(names(page)).toHaveText(["Mara Voss"]);
  await expect(page.getByText("1 von 3")).toBeVisible();
  // The choice stays after a reload; "Alle" clears it.
  await page.reload();
  await expect(names(page)).toHaveText(["Mara Voss"]);
  await filter.getByRole("button", { name: "Alle", exact: true }).click();
  await expect(names(page)).toHaveCount(3);
  await expect(filter.getByRole("button", { pressed: true })).toHaveText("Alle");
});

test("Charaktere: Kurztext in den Figur-Einstellungen, „Automatisch“ setzt zurück", async ({ page, open }) => {
  const mara = await charId("Mara Voss");
  const own = "Hütet das Licht auf Süderoog";
  const card = () => page.getByRole("article").filter({ hasText: "Mara Voss" });
  try {
    await open(`/char/${mara}/settings`);
    const field = page.getByRole("textbox", { name: "Kurztext unter der Karte" });
    await expect(field).toHaveValue("");
    await expect(field).toHaveAttribute("placeholder", /^Leuchtturmwärterin auf einer sturmumtosten Hallig/);
    await field.fill(own);
    await expect(page.getByText(`${[...own].length} / 160`)).toBeVisible();
    await page.getByRole("button", { name: "Speichern" }).click();
    await expect(page.getByRole("status").filter({ hasText: /^Gespeichert$/ })).toBeVisible();
    await expect.poll(async () => (await api<{ info: { tagline: string | null } }>("GET", `/characters/${mara}`)).info.tagline).toBe(own);

    await open("/chars");
    await expect(card().locator(".s")).toHaveText(own);
    // The search finds the own text too.
    await page.getByRole("textbox", { name: "Charaktere durchsuchen" }).fill("suderoog");
    await expect(names(page)).toHaveText(["Mara Voss"]);

    await open(`/char/${mara}/settings`);
    await expect(field).toHaveValue(own);
    await page.getByRole("button", { name: "Automatisch" }).click();
    await expect(field).toHaveValue("");
    await expect(page.getByRole("button", { name: "Automatisch" })).toBeDisabled();
    await page.getByRole("button", { name: "Speichern" }).click();
    await expect.poll(async () => (await api<{ info: { tagline: string | null } }>("GET", `/characters/${mara}`)).info.tagline).toBeNull();
    await open("/chars");
    await expect(card().locator(".s")).toHaveText(/^Leuchtturmwärterin auf einer sturmumtosten Hallig/);
  } finally {
    const { card: c } = await api<{ card: Record<string, unknown> }>("GET", `/characters/${mara}`);
    await api("PUT", `/characters/${mara}`, { ...c, tagline: null });
  }
});

test("Charaktere: drei Ansichten, gespeichert und nach dem Neuladen noch da", async ({ page, open }) => {
  await open("/chars");
  const view = page.getByRole("group", { name: "Ansicht" });
  await expect(view.getByRole("button", { name: "Poster" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".grid-chars.lib-poster")).toBeVisible();

  await view.getByRole("button", { name: "Liste" }).click();
  await expect(page.locator(".grid-chars.lib-list")).toBeVisible();
  await expect.poll(layoutOf).toBe("list");
  // The row shows the tags and when the last chat was.
  const row = page.getByRole("article").filter({ hasText: "Mara Voss" });
  await expect(row.locator(".lib-tag")).toHaveText(["Abenteuer", "Nordsee", "Slow Burn"]);
  await expect(row.locator(".lib-when")).toHaveText(/^(heute|gestern|\d+\.)/);
  // The star and the menu work in the list as well.
  await row.getByRole("button", { name: "Mara Voss als Favorit" }).click();
  await expect(row.getByRole("button", { name: "Mara Voss als Favorit" })).toHaveAttribute("aria-pressed", "true");
  await row.getByRole("button", { name: "Mara Voss: Aktionen zeigen" }).click();
  await expect(page.getByRole("group", { name: "Aktionen für Mara Voss" }).getByRole("link", { name: "Einstellungen" })).toBeVisible();
  await page.keyboard.press("Escape");

  await page.reload();
  await expect(page.getByRole("group", { name: "Ansicht" }).getByRole("button", { name: "Liste" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".grid-chars.lib-list")).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem("rp-library-layout"))).toBe("list");

  await page.getByRole("group", { name: "Ansicht" }).getByRole("button", { name: "Kompakt" }).click();
  await expect(page.locator(".grid-chars.lib-compact")).toBeVisible();
  await expect.poll(layoutOf).toBe("compact");
  await expect(names(page)).toHaveCount(3);
});

test("Strg K: Ansicht der Charaktere und Kurztext", async ({ page, open }) => {
  const moos = await charId("Moos");
  await open("/");
  await page.keyboard.press("Control+k");
  await expect(palette(page)).toBeVisible();
  await page.keyboard.type("charaktere als liste");
  await palette(page).getByRole("option", { name: /Charaktere als Liste/ }).click();
  await expect(page).toHaveURL(/#\/chars$/);
  await expect(page.locator(".grid-chars.lib-list")).toBeVisible();
  await expect.poll(layoutOf).toBe("list");

  // "Ansicht" points at the switch.
  await page.keyboard.press("Control+k");
  await page.keyboard.type("ansicht charaktere");
  await palette(page).getByRole("option", { name: /Ansicht der Charaktere/ }).click();
  await expect(page.getByRole("group", { name: "Ansicht" }).getByRole("button", { name: "Liste" })).toBeFocused();

  // The short text of the character on screen.
  await open(`/char/${moos}/chats`);
  await page.keyboard.press("Control+k");
  await page.keyboard.type("kurztext");
  await palette(page).getByRole("option", { name: /Kurztext unter der Karte/ }).first().click();
  await expect(page).toHaveURL(new RegExp(`#/char/${moos}/settings$`));
  await expect(page.getByRole("textbox", { name: "Kurztext unter der Karte" })).toBeFocused();
});
