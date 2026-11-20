// Settings: General (persona), Appearance (themes, start layout), Logs; Ctrl+K search into settings.

import { api, setLayout } from "./api";
import { expect, snap, test } from "./fixtures";

test.afterEach(async () => {
  await setLayout("bisher");
});

test("Einstellungen: Allgemein", async ({ page, open }) => {
  await open("/settings");
  const tabs = page.getByRole("navigation", { name: "Bereiche der Einstellungen" });
  await expect(tabs.getByRole("link", { name: "Allgemein" })).toHaveAttribute("aria-current", "page");
  await expect(page.getByRole("heading", { name: "Du spielst als", level: 2 })).toBeVisible();
  await expect(page.getByText(/wird in jeder Karte „User“/)).toBeVisible();
  await expect(page.getByRole("link", { name: "Personas verwalten" })).toBeVisible();
  await snap(page, "settings-general");
  await page.getByRole("link", { name: "Bearbeiten" }).click();
  await expect(page).toHaveURL(/#\/personas\/\d+$/);
  await expect(page.getByRole("heading", { name: "User", level: 2 })).toBeVisible();
});

test("Einstellungen: Darstellung (Farbschema, Start-Layout)", async ({ page, open, theme }) => {
  await open("/settings");
  await expect(page.getByRole("heading", { name: "Darstellung", level: 2 })).toBeVisible();
  const themes = page.getByRole("group", { name: "Farbschema" });
  await expect(themes.getByRole("button")).toHaveCount(6);
  const current = theme === "papier" ? "Papier" : "Graphit";
  await expect(themes.getByRole("button", { name: `Farbschema ${current}` })).toHaveAttribute("aria-pressed", "true");
  await themes.getByRole("button", { name: "Farbschema Mitternacht" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "mitternacht");
  await expect(themes.getByRole("button", { name: "Farbschema Mitternacht" })).toHaveAttribute("aria-pressed", "true");

  const layouts = page.getByRole("group", { name: "Start-Layout" });
  await expect(layouts.getByRole("button")).toHaveCount(5);
  await layouts.getByRole("button", { name: "Start-Layout Bühne" }).click();
  await expect(layouts.getByRole("button", { name: "Start-Layout Bühne" })).toHaveAttribute("aria-pressed", "true");
  await page.goto(page.url().replace(/#.*$/, "#/"));
  await expect(page.getByRole("region", { name: "Weiterlesen" })).toBeAttached();
});

test("Einstellungen: Logs", async ({ page, open }) => {
  // Quick successful requests are not logged; a failed one is (searched for below).
  await api("GET", "/characters/987654321").catch(() => {});
  await open("/settings");
  await page.getByRole("navigation", { name: "Bereiche der Einstellungen" }).getByRole("link", { name: "Logs" }).click();
  await expect(page).toHaveURL(/#\/settings\/logs$/);
  const filter = page.getByRole("group", { name: "Filter" });
  await expect(filter.getByRole("button", { name: "Alle" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("searchbox", { name: "Im Log suchen" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Exportieren", exact: true })).toBeEnabled();
  await expect(page.getByRole("region", { name: /^Heute/ }).getByRole("listitem").first()).toBeVisible();
  // What the log keeps at most, in one line under the tools.
  await expect(page.getByText("Behält höchstens 7 Tage bzw. 20 000 Einträge (8 MB); Wiederholungen werden zusammengefasst.")).toBeVisible();
  await snap(page, "settings-logs", [page.getByRole("region", { name: /^Heute/ }), page.locator(".log-count")]);

  await filter.getByRole("button", { name: "Server" }).click();
  await expect(filter.getByRole("button", { name: "Server" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("region", { name: /^Heute/ }).getByRole("listitem").first()).toContainText("Server");
  await page.getByRole("searchbox", { name: "Im Log suchen" }).fill("987654321");
  await expect(page.getByRole("region", { name: /^Heute/ }).getByRole("listitem").first()).toContainText("/api/characters/987654321");
});

test("Strg K: Suche nach einer Einstellung öffnet die richtige Stelle", async ({ page, open }) => {
  await open("/chars");
  await page.keyboard.press("Control+k");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("combobox").fill("dunkel");
  await expect(dialog.getByRole("option").first()).toContainText("Farbschema");
  await snap(page, "palette");
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect(page).toHaveURL(/#\/settings$/);
  await expect(page.getByRole("group", { name: "Farbschema" })).toBeInViewport();

  // A rough description finds a persona setting; the current choice there gets the focus.
  await page.keyboard.press("Control+k");
  await dialog.getByRole("combobox").fill("vorrang");
  await expect(dialog.getByRole("option").first()).toContainText("Wenn eine Karte dich anders beschreibt");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/personas\/\d+$/);
  await expect(page.getByRole("radio", { name: /^Meine Persona gilt/ })).toBeFocused();
});

test("Strg K: „Logs“ hebt die Log-Liste hervor", async ({ page, open }) => {
  // Was a bug until R3a: the command jumped to data-spot="logs", which no element had, so the page
  // opened but nothing was pointed at.
  await open("/settings");
  await page.keyboard.press("Control+k");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  await dialog.getByRole("combobox").fill("logs");
  await expect(dialog.getByRole("option").first()).toContainText("Logs");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/settings\/logs$/);
  await expect(page.locator(".spot-on")).toBeInViewport();
});

test("Logs live: eine Wiederholung zählt den Eintrag ohne Neuladen hoch", async ({ page, open }) => {
  // The server merges an equal entry within 60 s into the one before it ("… (×2)"); the live view
  // must show the new count, not only after a reload.
  const message = `Testeintrag ${Date.now().toString(36)}`;
  const post = () => api("POST", "/logs", { events: [{ ts: Date.now(), level: "info", category: "test", message }] });
  await open("/settings/logs");
  // The page sends its own start events within FLUSH_MS (2 s, log.ts); any entry between the two
  // posts would end the merge, so let those go out first.
  await page.waitForTimeout(2500);
  const rows = page.getByRole("region", { name: /^Heute/ }).getByRole("listitem").filter({ hasText: message });
  await post();
  await expect(rows).toHaveCount(1);
  await expect(rows).not.toContainText("(×");
  await post();
  await expect(rows).toContainText(`${message} (×2)`);
  await expect(rows).toHaveCount(1);
});
