// K4 ordering the library: sort (remembered), star, favourites only, own tags from the card editor,
// filter by tag, and the selection mode (tag, favourite, export, delete with undo).

import type { Page } from "@playwright/test";
import { api, charId, library } from "./api";
import { expect, test } from "./fixtures";
import { withoutSavePicker } from "./save-picker";

// The plain download (as on a phone); the save dialog has its own tests in save-dialog.spec.ts.
test.beforeEach(({ page }) => withoutSavePicker(page));

const TAG = "Seefahrt";
const names = (page: Page) => page.locator(".grid-chars .cc .body b");

/** Back to the state the other tests expect: no favourites, no own tags, nothing deleted. */
async function reset() {
  const ids = (await library()).characters.map((c) => c.id);
  if (ids.length === 0) return;
  await api("POST", "/characters/bulk", { ids, action: "unfavorite" });
  for (const id of ids) await api("PUT", `/characters/${id}/tags`, { tags: [] });
}

test.afterEach(reset);

test("Bibliothek: sortieren, Stern, Favoriten, Tags, Filter, Mehrfachauswahl", async ({ page, open }) => {
  await reset();
  const ilse = await charId("Kapitän Ilse Brandt");
  await open("/chars");

  // Sort by name, and the choice stays after a reload.
  const sort = page.getByRole("combobox", { name: "Sortieren" });
  await expect(sort).toHaveValue("recent");
  await sort.selectOption("name");
  await expect(names(page)).toHaveText(["Kapitän Ilse Brandt", "Mara Voss", "Moos"]);
  await sort.selectOption("new");
  await expect(names(page)).toHaveText(["Moos", "Kapitän Ilse Brandt", "Mara Voss"]);
  await sort.selectOption("name");
  await page.reload();
  await expect(page.getByRole("combobox", { name: "Sortieren" })).toHaveValue("name");
  await expect(names(page)).toHaveText(["Kapitän Ilse Brandt", "Mara Voss", "Moos"]);

  // The star on the poster card, and "only favourites".
  const mara = page.locator(".cc").filter({ hasText: "Mara Voss" });
  await mara.hover();
  const star = page.getByRole("button", { name: "Mara Voss als Favorit" });
  await star.click();
  await expect(star).toHaveAttribute("aria-pressed", "true");
  const favs = page.getByRole("button", { name: "Favoriten" });
  await favs.click();
  await expect(favs).toHaveAttribute("aria-pressed", "true");
  await expect(names(page)).toHaveText(["Mara Voss"]);
  await expect(page.getByText("1 von 3")).toBeVisible();
  await favs.click();
  await expect(names(page)).toHaveCount(3);
  expect((await library()).characters.find((c) => c.name === "Mara Voss")).toMatchObject({ favorite: true });

  // An own tag from the card editor (field "Tags"), saved at once.
  await open(`/char/${ilse}/settings`);
  const tags = page.getByRole("combobox", { name: "Tags", exact: true });
  await tags.fill(TAG);
  await tags.press("Enter");
  await expect(page.locator(".own-tags .tag-chip")).toHaveText([TAG]);
  await expect.poll(async () => (await api<{ tags: { name: string; count: number }[] }>("GET", "/tags")).tags).toEqual([{ name: TAG, count: 1 }]);

  // Filter by the tag: only Ilse.
  await open("/chars");
  const chip = page.getByRole("group", { name: "Nach Tag filtern" }).getByRole("button", { name: new RegExp(`^${TAG}`) });
  await chip.click();
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expect(names(page)).toHaveText(["Kapitän Ilse Brandt"]);
  await page.getByRole("group", { name: "Nach Tag filtern" }).getByRole("button", { name: "Alle", exact: true }).click();
  await expect(names(page)).toHaveCount(3);

  // Selection mode: pick Mara and Moos, give both the tag.
  await page.getByRole("button", { name: "Auswählen", exact: true }).click();
  await page.getByRole("checkbox", { name: "Mara Voss auswählen" }).check();
  await page.getByRole("checkbox", { name: "Moos auswählen" }).check();
  const bar = page.getByRole("toolbar", { name: "Auswahl" });
  await expect(bar.getByRole("status")).toHaveText("2 ausgewählt");
  await bar.getByRole("button", { name: "Tag hinzufügen oder entfernen" }).click();
  await bar.getByRole("combobox", { name: "Tag" }).fill(TAG);
  await bar.getByRole("button", { name: "Hinzufügen" }).click();
  await expect(page.locator(".toast")).toContainText(`Tag „${TAG}“ bei 2 Karten hinzugefügt`);
  await expect(chip).toContainText("3");

  // Favourite for both, then export them as one ZIP.
  await bar.getByRole("button", { name: "Als Favorit" }).click();
  await expect.poll(async () => (await library()).characters.filter((c) => (c as { favorite?: boolean }).favorite).length).toBe(2);
  await bar.getByRole("button", { name: "Exportieren" }).click();
  const download = page.waitForEvent("download");
  await bar.getByRole("button", { name: "PNG" }).click();
  expect((await download).suggestedFilename()).toMatch(/^characters-.*\.zip$/);

  // Delete both, then undo it.
  await bar.getByRole("button", { name: "Löschen" }).click();
  await expect(page.locator(".toast")).toContainText("2 Karten gelöscht");
  await expect(names(page)).toHaveText(["Kapitän Ilse Brandt"]);
  await page.locator(".toast").getByRole("button", { name: "Rückgängig" }).click();
  await expect(names(page)).toHaveCount(3);
  await expect(bar).toHaveCount(0);

  // Untag in one go: only Ilse's card tag of the same name is left in the filter.
  const ids = (await library()).characters.map((c) => c.id);
  await api("POST", "/characters/bulk", { ids, action: "untag", tag: TAG.toLowerCase() });
  await expect(chip).toHaveText(new RegExp(`^${TAG}\\s*1$`));
});
