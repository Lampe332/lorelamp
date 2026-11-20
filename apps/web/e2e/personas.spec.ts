// Personas: list, creating a new one, and the switcher (sidebar on a desktop, sheet on a phone).

import { api, library } from "./api";
import { expect, isPhone, snap, test } from "./fixtures";
import { SECOND_PERSONA } from "./global-setup";

const NEW_NAME = "Neue Testperson";

test.afterEach(async () => {
  const lib = await library();
  const user = lib.personas.find((p) => p.name === "User");
  if (user && lib.active_persona_id !== user.id) await api("PUT", `/personas/${user.id}/activate`);
  for (const p of lib.personas.filter((p) => p.name === NEW_NAME)) await api("DELETE", `/personas/${p.id}`);
});

test("Personas: Liste", async ({ page, open }) => {
  await open("/personas");
  await expect(page.getByRole("heading", { name: "Personas", level: 1 })).toBeVisible();
  const list = page.getByRole("navigation", { name: "Alle Personas" });
  await expect(list.getByRole("link")).toHaveCount(2);
  await expect(list.getByRole("link", { name: /^User .*Aktiv$/ })).toBeVisible();
  await expect(list.getByRole("link", { name: new RegExp(`^${SECOND_PERSONA} sie/ihr`) })).toBeVisible();
  await snap(page, "personas");
  await list.getByRole("link", { name: new RegExp(`^${SECOND_PERSONA}`) }).click();
  await expect(page.getByRole("heading", { name: SECOND_PERSONA, level: 2 })).toBeVisible();
  await expect(page.getByRole("textbox", { name: /^Freie Beschreibung/ })).toHaveValue("Eine Reisende.");
});

test("Personas: neue Persona anlegen", async ({ page, open }) => {
  await open("/personas");
  await page.getByRole("link", { name: "Neue Persona" }).click();
  await expect(page).toHaveURL(/#\/personas\/new$/);
  await page.getByRole("textbox", { name: /^Name/ }).fill(NEW_NAME);
  await page.getByRole("textbox", { name: /^Aussehen/ }).fill("Grauer Mantel, rote Mütze.");
  await expect(page.getByText("Noch nicht angelegt")).toBeVisible();
  await page.getByRole("button", { name: "Anlegen" }).click();
  await expect(page).toHaveURL(/#\/personas\/\d+$/);
  await expect(page.getByRole("heading", { name: NEW_NAME, level: 2 })).toBeVisible();
  // Phones show list and editor one after the other: back to the list there.
  if (isPhone(page)) await page.getByRole("link", { name: "Zurück zu allen Personas" }).click();
  const list = page.getByRole("navigation", { name: "Alle Personas" });
  await expect(list.getByRole("link", { name: new RegExp(`^${NEW_NAME}`) })).toBeVisible();
  const saved = (await library()).personas.find((p) => p.name === NEW_NAME);
  expect(saved).toBeTruthy();
});

test("Personas: Umschalter", async ({ page, open }) => {
  await open("/");
  const button = isPhone(page)
    ? page.locator("header.top").getByRole("button", { name: "User" })
    : page.getByRole("complementary").getByRole("button", { name: "Du spielst als User" });
  await button.click();
  const menu = page.getByRole("menu");
  await expect(menu.getByRole("menuitemradio", { name: /^User/ })).toHaveAttribute("aria-checked", "true");
  await menu.getByRole("menuitemradio", { name: new RegExp(`^${SECOND_PERSONA}`) }).click();
  await expect(menu).toHaveCount(0);
  const after = isPhone(page)
    ? page.locator("header.top").getByRole("button", { name: SECOND_PERSONA })
    : page.getByRole("complementary").getByRole("button", { name: `Du spielst als ${SECOND_PERSONA}` });
  await expect(after).toBeVisible();
  const lib = await library();
  expect(lib.personas.find((p) => p.id === lib.active_persona_id)?.name).toBe(SECOND_PERSONA);
});
