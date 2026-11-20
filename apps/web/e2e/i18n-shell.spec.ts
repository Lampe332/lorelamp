// The app's frame in English, Spanish and Russian holds no German (dictionary `shell`, see
// docs/i18n.md): sidebar, the quick actions' window (Ctrl+K), the phone's bars and its sheet "Mehr",
// the tablet rail, and Settings → Allgemein with the language choice. Needs no model answer.
// A group of Q7b adds its own `i18n-<group>.spec.ts` with a table like this one.

import type { Page } from "@playwright/test";
import { expect } from "./fixtures";
import { leftoverTests } from "./leftovers";

/** The quick actions' window with its list of commands. */
async function palette(page: Page) {
  const dialog = page.locator(".cmd-dialog");
  await expect(dialog.locator(".cmd-item").first()).toBeVisible();
  return dialog;
}

/** The phone's sheet "Mehr", slid in. */
async function more(page: Page) {
  await page.locator("nav.tabbar button.ph-tab").click();
  const sheet = page.locator("dialog.ph-sheet");
  await expect(sheet).toBeVisible();
  return sheet;
}

/** In the quick actions, the chats' titles and their last lines are the user's own text. */
const CHAT_TEXT = ['.cmd-part[data-group="Weiter"] .cmd-hint', '.cmd-part[data-group="Chats"] .cmd-hint'];

leftoverTests("shell", [
  { name: "Seitenleiste", form: "desktop", hash: "/", region: (page) => page.locator("aside.side") },
  {
    name: "Strg K",
    form: "desktop",
    hash: "/",
    region: async (page) => {
      await page.keyboard.press("Control+k");
      return palette(page);
    },
    skip: CHAT_TEXT,
  },
  // The frame and the tab Allgemein; the active persona's description is the user's own text.
  { name: "Einstellungen › Allgemein", form: "desktop", hash: "/settings", region: (page) => page.locator("body"), skip: [".sp-text"] },

  { name: "Kopf und untere Leiste", form: "phone", hash: "/", region: (page) => page.locator("header.top, nav.tabbar") },
  { name: "Mehr", form: "phone", hash: "/", region: more },
  {
    name: "Suchen über Mehr",
    form: "phone",
    hash: "/",
    region: async (page) => {
      await (await more(page)).locator("button.ph-row").click();
      return palette(page);
    },
    skip: CHAT_TEXT,
  },
  { name: "Einstellungen › Allgemein", form: "phone", hash: "/settings", region: (page) => page.locator("body"), skip: [".sp-text"] },

  { name: "Leiste", form: "tablet", hash: "/", region: (page) => page.locator("aside.tb-rail") },
  { name: "Einstellungen › Allgemein", form: "tablet", hash: "/settings", region: (page) => page.locator("body"), skip: [".sp-text"] },
]);
