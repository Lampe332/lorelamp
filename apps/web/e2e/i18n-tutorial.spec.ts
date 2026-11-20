// The welcome tour (Q9, dictionary `tutorial`) in English, Spanish and Russian holds no German: every
// step on a desktop, a phone and a tablet, the tour put aside at the edge, and its row in
// Settings → Allgemein. The tour's step is put back as it was found.

import type { Page } from "@playwright/test";
import { api } from "./api";
import { expect, test } from "./fixtures";
import { leftoverTests, type Screen } from "./leftovers";

type Step = "welcome" | "model" | "figure" | "persona" | "chat";

let before = "done";
test.beforeAll(async () => {
  before = (await api<{ step: string }>("GET", "/tutorial")).step;
});
test.afterAll(() => api("PUT", "/tutorial", { step: before }));

/** Opens the tour at `step` (the page is already open) and gives back its window. */
async function at(page: Page, step: Step) {
  await api("PUT", "/tutorial", { step });
  await page.reload();
  const tour = page.getByRole("dialog");
  await expect(tour).toBeVisible();
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  return tour;
}

const STEPS: Step[] = ["welcome", "model", "figure", "persona", "chat"];
const screens: Screen[] = [];
for (const form of ["desktop", "phone", "tablet"] as const) {
  for (const step of STEPS) {
    screens.push({
      name: `Einführung ${step}`,
      form,
      hash: "/",
      region: (page) => at(page, step),
      skip: [".tu-fig b", ".tu-name input"],
      keep: ["Lampe", "Chub"],
    });
  }
}
screens.push({
  name: "Einführung am Rand",
  form: "desktop",
  hash: "/",
  region: async (page) => {
    await at(page, "figure");
    await page.keyboard.press("Escape");
    return page.locator(".tu-dock");
  },
});
screens.push({
  name: "Einführung am Rand",
  form: "phone",
  hash: "/",
  region: async (page) => {
    await at(page, "figure");
    await page.keyboard.press("Escape");
    return page.locator(".tu-dock");
  },
});
for (const form of ["desktop", "phone"] as const) {
  screens.push({
    name: "Einstellungen › Einführung",
    form,
    hash: "/settings",
    region: async (page) => {
      await api("PUT", "/tutorial", { step: "done" });
      return page.locator('[data-spot="tutorial"]');
    },
  });
}

leftoverTests("tutorial", screens);
