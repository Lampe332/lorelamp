// Characters page (poster grid, click menu) and the character page with its two tabs.

import { api, charId } from "./api";
import { expect, isPhone, snap, test } from "./fixtures";

test("Charaktere: Raster und Klick-Menü", async ({ page, open }) => {
  await open("/chars");
  await expect(page.getByRole("heading", { name: "Charaktere", level: 1 })).toBeVisible();
  await expect(page.getByText("3 Karten")).toBeVisible();
  for (const name of ["Mara Voss", "Kapitän Ilse Brandt", "Moos"]) {
    await expect(page.getByRole("button", { name: `${name}: Aktionen zeigen` })).toBeVisible();
  }
  // "Zuletzt gechattet" (the default) follows the other tests' chats; the screenshot needs a fixed order.
  await page.getByRole("combobox", { name: "Sortieren" }).selectOption("name");
  await snap(page, "chars");

  const open1 = page.getByRole("button", { name: "Mara Voss: Aktionen zeigen" });
  await open1.click();
  await expect(open1).toHaveAttribute("aria-expanded", "true");
  const menu = page.getByRole("group", { name: "Aktionen für Mara Voss" });
  await expect(menu.getByRole("button", { name: "Weiter" })).toBeVisible();
  await expect(menu.getByRole("button", { name: "Neuer Chat" })).toBeVisible();
  await expect(menu.getByRole("link", { name: "Einstellungen" })).toBeVisible();
  await menu.getByRole("button", { name: "Weiter" }).click();
  await expect(page).toHaveURL(/#\/chat\/\d+$/);
  await expect(page.getByRole("heading", { name: "Mara Voss", level: 1 })).toBeVisible();

  await open("/chars");
  await page.getByRole("button", { name: "Moos: Aktionen zeigen" }).click();
  await page.getByRole("group", { name: "Aktionen für Moos" }).getByRole("link", { name: "Einstellungen" }).click();
  await expect(page).toHaveURL(new RegExp(`#/char/${await charId("Moos")}`));
  await expect(page.getByRole("heading", { name: "Moos", level: 1 })).toBeVisible();
});

test("Figur-Seite: Tab Chats", async ({ page, open }) => {
  const id = await charId("Mara Voss");
  await open(`/char/${id}/chats`);
  const tabs = page.getByRole("navigation", { name: "Bereiche dieser Figur" });
  await expect(tabs.getByRole("link", { name: "Chats" })).toHaveAttribute("aria-current", "page");
  await expect(page.getByRole("heading", { name: "1 Chat", level: 2 })).toBeVisible();
  await expect(page.getByRole("button", { name: "Neuer Chat" })).toBeVisible();
  await expect(page.getByRole("link", { name: /^Karte nur für .* anpassen$/ })).toBeVisible();
  // The side panel about the character is a desktop element; phones show only the chat list.
  if (!isPhone(page)) {
    await expect(page.getByRole("complementary", { name: "Über Mara Voss" }).getByRole("listitem")).toHaveText(["Abenteuer", "Nordsee", "Slow Burn"]);
  }
  await snap(page, "char-chats");
  await tabs.getByRole("link", { name: "Einstellungen" }).click();
  await expect(page).toHaveURL(new RegExp(`#/char/${id}/settings$`));
  await expect(page.getByRole("heading", { name: "Profil", level: 2 })).toBeVisible();
});

test("Figur-Seite: „Neuer Chat“ öffnet den neuen Chat", async ({ page, open }) => {
  const id = await charId("Mara Voss");
  const before = (await api<{ chats: { id: number }[] }>("GET", `/characters/${id}/chats`)).chats.map((c) => c.id);
  await open(`/char/${id}/chats`);
  await page.getByRole("button", { name: "Neuer Chat" }).first().click();
  await expect(page).toHaveURL(/#\/chat\/\d+$/);
  const chatId = Number(/#\/chat\/(\d+)$/.exec(page.url())![1]);
  try {
    // A chat that did not exist before, and the chat screen itself is open.
    expect(before).not.toContain(chatId);
    await expect(page.getByRole("heading", { name: "Mara Voss", level: 1 })).toBeVisible();
    await expect(page.locator(".composer textarea")).toBeVisible();
  } finally {
    await api("DELETE", `/chats/${chatId}`);
  }
});

test("Figur-Seite: Tab Einstellungen mit „Du in dieser Geschichte“", async ({ page, open }) => {
  const id = await charId("Mara Voss");
  try {
    await open(`/char/${id}/settings`);
    await expect(page.getByRole("textbox", { name: "Name", exact: true })).toHaveValue("Mara Voss");
    await expect(page.getByRole("radiogroup", { name: "Standard-Persona für neue Chats" }).getByRole("radio")).toHaveCount(3);
    await expect(page.getByRole("button", { name: "Speichern" })).toBeDisabled();
    await snap(page, "char-settings");

    await expect(page.getByRole("heading", { name: "Du in dieser Geschichte", level: 2 })).toBeVisible();
    const fit = page.locator('[data-spot="user-fit"]');
    await expect(fit.getByRole("status").first()).toContainText("Passt");
    const compare = page.getByRole("radiogroup", { name: "Verglichen mit" });
    await compare.getByRole("radio", { name: /Testperson Zwei/ }).click();
    await expect(compare.getByRole("radio", { name: /Testperson Zwei/ })).toBeChecked();
    await expect(fit.getByRole("status").first()).toContainText("Testperson Zwei");

    const rule = page.getByRole("radiogroup", { name: "Was im Chat gilt" });
    // Click the visible option text, as a user does (the native radio sits under its custom mark).
    const cardWins = rule.getByText("Die Karte gilt", { exact: true });
    await cardWins.evaluate((el) => el.scrollIntoView({ block: "center" }));
    await cardWins.click();
    await expect(rule.getByRole("radio", { name: /^Die Karte gilt/ })).toBeChecked();
    await expect.poll(async () => (await api<{ info: { user_priority: string | null } }>("GET", `/characters/${id}`)).info.user_priority).toBe("card");

    // Editing a field makes the form dirty; "Verwerfen" brings it back.
    const blurb = page.getByRole("textbox", { name: /^Kurzbeschreibung/ });
    await blurb.fill("Geändert");
    await expect(page.getByRole("button", { name: "Speichern" })).toBeEnabled();
    await page.getByRole("button", { name: "Verwerfen" }).click();
    await expect(blurb).toHaveValue("Leuchtturmwärterin auf einer sturmumtosten Hallig. SFW-Beispielkarte.");
  } finally {
    await api("PUT", `/characters/${id}/user-priority`, { priority: null });
  }
});
