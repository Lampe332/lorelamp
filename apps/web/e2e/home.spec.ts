// Start screen: all five layouts (all are permanent), the sidebar and its model status.

import { setLayout } from "./api";
import { expect, isPhone, modelStatus, snap, test } from "./fixtures";
import type { Page } from "@playwright/test";

test.afterAll(async () => {
  await setLayout("bisher");
});

const LAYOUTS: { id: string; check: (page: Page) => Promise<void> }[] = [
  {
    id: "bisher",
    check: async (page) => {
      await expect(page.getByRole("heading", { name: "Start", level: 1 })).toBeVisible();
      await expect(page.getByRole("link", { name: /^Zuletzt mit / })).toBeVisible();
      await expect(page.getByRole("region", { name: "Weitere Chats" })).toBeVisible();
      await expect(page.getByRole("region", { name: /^Charaktere/ }).getByRole("link", { name: /Mara Voss/ })).toBeVisible();
    },
  },
  {
    id: "dashboard",
    check: async (page) => {
      await expect(page.getByRole("heading", { name: "Start", level: 1 })).toBeVisible();
      await expect(page.getByRole("term").filter({ hasText: "Figuren" })).toBeVisible();
      await expect(page.getByRole("navigation", { name: "Schnellaktionen" }).getByRole("button", { name: "Neuer Chat" })).toBeVisible();
      await expect(page.getByRole("region", { name: /^Deine Figuren/ })).toBeVisible();
      await expect(page.getByRole("region", { name: "Letzte Messung" })).toBeAttached();
    },
  },
  {
    id: "bento",
    check: async (page) => {
      await expect(page.getByRole("heading", { name: "Start", level: 1 })).toBeVisible();
      await expect(page.getByRole("link", { name: /^Zuletzt mit / })).toBeVisible();
      await expect(page.getByRole("button", { name: "Start-Layout wechseln" })).toBeAttached();
      await expect(page.getByRole("list", { name: "Bald in der App" })).toBeAttached();
    },
  },
  {
    id: "buehne",
    check: async (page) => {
      const lead = page.getByRole("region", { name: "Zuletzt" });
      await expect(lead.getByRole("heading", { level: 1 })).toBeVisible();
      await expect(lead.getByRole("link", { name: "Weiterschreiben" })).toBeVisible();
      await expect(page.getByRole("region", { name: "Weiterlesen" })).toBeAttached();
    },
  },
  {
    id: "kommando",
    check: async (page) => {
      await expect(page.getByRole("heading", { name: "Was möchtest du tun?", level: 1 })).toBeVisible();
      await expect(page.getByRole("combobox", { name: /^Suchen: Figur, Chat, Einstellung/ })).toBeVisible();
      await expect(page.getByRole("region", { name: "Modell" })).toBeAttached();
      // The command field uses the same search as Ctrl+K.
      await page.getByRole("combobox", { name: /^Suchen: Figur, Chat, Einstellung/ }).fill("Mara");
      await expect(page.getByRole("option", { name: /Mara Voss/ }).first()).toBeVisible();
      await page.getByRole("combobox", { name: /^Suchen: Figur, Chat, Einstellung/ }).fill("");
    },
  },
];

for (const layout of LAYOUTS) {
  test(`Start-Layout ${layout.id}`, async ({ page, open }) => {
    await setLayout(layout.id);
    await open("/");
    await layout.check(page);
    await snap(page, `home-${layout.id}`);
  });
}

test("Seitenleiste: Modell-Status und Navigation", async ({ page, open }) => {
  await setLayout("bisher");
  await open("/");
  await expect(modelStatus(page)).toContainText("Modell bereit");
  if (isPhone(page)) {
    const bar = page.getByRole("navigation", { name: "Bereiche" });
    await bar.getByRole("link", { name: "Charaktere" }).click();
    await expect(page).toHaveURL(/#\/chars$/);
    await bar.getByRole("link", { name: "Einstellungen" }).click();
    await expect(page).toHaveURL(/#\/settings$/);
  } else {
    const side = page.getByRole("complementary");
    await expect(side.getByRole("navigation", { name: "Letzte Chats" }).getByRole("link")).toHaveCount(3);
    await side.getByRole("navigation", { name: "Spielen" }).getByRole("link", { name: "Charaktere" }).click();
    await expect(page).toHaveURL(/#\/chars$/);
    await side.getByRole("navigation", { name: "Welt" }).getByRole("link", { name: "Personas" }).click();
    await expect(page).toHaveURL(/#\/personas$/);
    await side.getByRole("navigation", { name: "App" }).getByRole("link", { name: "Einstellungen" }).click();
    await expect(page).toHaveURL(/#\/settings$/);
  }
});
