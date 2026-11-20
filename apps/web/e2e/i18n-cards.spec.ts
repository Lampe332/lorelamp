// Group B "cards" in English, Spanish and Russian holds no German (dictionary `cards`, see
// docs/i18n.md): the characters tab (cards, a card's actions, the selection bar and its steps, the
// list layout), a character's chats (row menu, selection, export dialog), the card editor with
// "Du in dieser Geschichte", the default persona and the picture link, the card export dialog, a
// chat's own card, the personas (list, editor, new one), the persona menus of the sidebar and the
// chat bar, and the tablet's figure pane. Needs no model answer.
//
// Desktop screens are read at 1280 px (the narrow end of a desktop window), phone screens at 375 px;
// none of them may scroll sideways. With RP_E2E_SHOTS each screen is saved as a picture there.

import type { Locator, Page } from "@playwright/test";
import { api, library } from "./api";
import { expect, test } from "./fixtures";
import { leftoverTests } from "./leftovers";

/** The user's own content and the cards' texts (the test cards are German) are not read: */
const CARD_TEXT = [
  "textarea",
  ".s", // a card's short text
  ".lib-tag", // a card's tags in the list layout
  '.tag-chip[data-log-label="Tag-Filter"]', // the tags of the cards in the tag filter
  ".about-blurb",
  ".about-tags",
  ".tb-pane-about",
  ".tb-pane-tags",
  ".l2", // the start of a chat's last message
  ".lib-tagline .input", // its placeholder is the card's automatic short text
  ".uf-vals", // what the card says about the user, in the card's words
  ".uf-proof ul",
  ".uf-rules ul",
  ".uf-theirs > span", // the persona's own words beside it
];
/** A persona's line in lists: its pronouns and the start of its description. */
const PERSONA_TEXT = [".pl-text small", ".pe-title small", ".pm-item:not(:has(.pm-ico)) .pm-text small", ".dp-opt:not(:has(.dp-none)) .dp-text small"];
const SKIP = [...CARD_TEXT, ...PERSONA_TEXT];

const charId = async (name: string) => {
  const c = (await library()).characters.find((x) => x.name === name);
  if (!c) throw new Error(`no test character ${name}`);
  return c.id;
};
const chatOf = async (name: string) => (await api<{ chats: { id: number }[] }>("GET", `/characters/${await charId(name)}/chats`)).chats[0].id;
const personaId = async (name: string) => {
  const p = (await library()).personas.find((x) => x.name === name);
  if (!p) throw new Error(`no test persona ${name}`);
  return p.id;
};

/** Desktop screens are read at 1280 px. */
async function narrow(page: Page) {
  if ((page.viewportSize()?.width ?? 0) > 1280) await page.setViewportSize({ width: 1280, height: 800 });
}

/** Moves to another page of the app without a reload. */
async function go(page: Page, hash: string) {
  await page.evaluate((h) => (location.hash = h), hash);
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
}

/** No sideways scrolling, and with RP_E2E_SHOTS a picture of the screen. */
async function shown(page: Page, name: string, region: Locator): Promise<Locator> {
  await expect(region.first()).toBeVisible();
  // Polled: a sheet or header may stick out for a moment while it slides in.
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth), { message: `${name} scrolls sideways` }).toBeLessThanOrEqual(0);
  const dir = process.env.RP_E2E_SHOTS;
  if (dir) {
    const lang = await page.locator("html").getAttribute("lang");
    const form = await page.locator("html").getAttribute("data-form");
    const width = page.viewportSize()?.width ?? 0;
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: `${dir}/${lang}-${width}-${form === "tablet" ? "tablet-" : ""}${name}.png` });
  }
  return region;
}

const view = (page: Page) => page.locator("div.view").first();
const charPage = (page: Page) => page.locator(".chat-main");

leftoverTests("cards", [
  // ---- desktop
  {
    name: "Charaktere",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await expect(page.locator(".cc").first()).toBeVisible();
      return shown(page, "library", view(page));
    },
    skip: SKIP,
  },
  {
    name: "Charaktere: Aktionen einer Karte",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await page.locator(".cc-open").first().click();
      await expect(page.locator(".cc-menu")).toBeVisible();
      return shown(page, "library-card", view(page));
    },
    skip: SKIP,
  },
  {
    name: "Charaktere: Auswahl, Tag und Export",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await page.locator(".title-row .actions .btn.icon-sm").click();
      await page.locator(".cc-check input").first().check();
      const bar = page.locator(".lib-selbar");
      await expect(bar.getByRole("status")).toBeVisible();
      await shown(page, "library-select", view(page));
      // The two second steps of the bar: tag, export (both are read after the bar itself).
      await bar.locator(".btn.icon-sm").nth(1).click();
      await expect(bar.locator("form.sel-step")).toBeVisible();
      await shown(page, "library-select-tag", view(page));
      await bar.locator(".sel-step .btn.ghost").click();
      await bar.locator(".btn.icon-sm").nth(2).click();
      await expect(bar.locator("div.sel-step")).toBeVisible();
      return shown(page, "library-select-export", view(page));
    },
    skip: SKIP,
  },
  {
    name: "Charaktere: Liste",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      // The layout is a server setting; every test starts with the one from before (see the hooks below).
      await page.locator(".lib-seg button").nth(2).click();
      await expect(page.locator(".grid-chars.lib-list")).toBeVisible();
      await expect(page.locator(".lib-when").first()).toBeVisible();
      return shown(page, "library-list", view(page));
    },
    skip: SKIP,
  },
  {
    name: "Figur › Chats",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/char/${await charId("Mara Voss")}/chats`);
      await expect(page.locator(".chat-row").first()).toBeVisible();
      return shown(page, "char-chats", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Figur › Chats: Menü einer Zeile",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/char/${await charId("Mara Voss")}/chats`);
      await page.locator('.row-actions button[aria-haspopup="menu"]').first().click();
      return shown(page, "char-chats-menu", page.locator(".row-menu"));
    },
    skip: SKIP,
  },
  {
    name: "Figur › Chats: Auswahl und Export als ZIP",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/char/${await charId("Mara Voss")}/chats`);
      await page.locator(".title-row .actions .btn.icon-sm").click();
      await page.locator(".chat-row input[type=checkbox]").first().check();
      await expect(page.locator(".selbar").getByRole("status")).toBeVisible();
      await shown(page, "char-chats-select", charPage(page));
      await page.locator(".selbar .btn.primary").click();
      const dialog = page.locator("dialog.xf");
      await expect(dialog).toBeVisible();
      return shown(page, "char-chats-export", page.locator(".selbar, dialog.xf"));
    },
    skip: SKIP,
  },
  {
    name: "Figur › Einstellungen (Bild-Link)",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/char/${await charId("Moos")}/settings`);
      await expect(page.locator('[data-spot="user-fit"] .uf-head')).toBeVisible();
      await page.locator('[data-spot="char-picture"] button[aria-expanded]').click();
      await expect(page.locator(".pic-link")).toBeVisible();
      return shown(page, "char-settings", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Figur › Einstellungen: Du in dieser Geschichte",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/char/${await charId("Mara Voss")}/settings`);
      const fit = page.locator('[data-spot="user-fit"]');
      await expect(fit.locator(".uf-head")).toBeVisible();
      // Fold out the evidence and the card's rules, if there are any.
      for (const d of await fit.locator("details").all()) await d.locator("summary").click();
      await fit.scrollIntoViewIfNeeded();
      return shown(page, "char-userfit", fit);
    },
    skip: SKIP,
  },
  {
    name: "Karte exportieren",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/char/${await charId("Mara Voss")}/settings`);
      await page.locator('.head-tools button[aria-haspopup="dialog"]').click();
      return shown(page, "card-export", page.locator("dialog.xf"));
    },
    skip: SKIP,
  },
  {
    name: "Karte nur für diesen Chat",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/chat/${await chatOf("Mara Voss")}/card`);
      await expect(page.locator(".own-note")).toBeVisible();
      return shown(page, "chat-card", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Personas",
    form: "desktop",
    hash: "/personas",
    region: async (page) => {
      await narrow(page);
      await expect(page.locator(".pe")).toBeVisible();
      return shown(page, "personas", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Persona bearbeiten (nicht aktiv)",
    form: "desktop",
    hash: "/personas",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/personas/${await personaId("Testperson Zwei")}`);
      await expect(page.locator(".pe-danger")).toBeVisible();
      return shown(page, "persona-edit", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Neue Persona",
    form: "desktop",
    hash: "/personas/new",
    region: async (page) => {
      await narrow(page);
      await expect(page.locator(".pe")).toBeVisible();
      return shown(page, "persona-new", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Persona-Menü der Seitenleiste",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await page.locator("aside.side .persona-btn").click();
      return shown(page, "persona-menu", page.locator(".pm"));
    },
    skip: SKIP,
  },
  {
    name: "Persona eines Chats",
    form: "desktop",
    hash: "/chars",
    region: async (page) => {
      await narrow(page);
      await go(page, `#/chat/${await chatOf("Mara Voss")}`);
      await page.locator(".chat-persona").click();
      await expect(page.locator(".pm")).toBeVisible();
      return shown(page, "chat-persona", page.locator(".chat-persona, .pm"));
    },
    skip: SKIP,
  },

  // ---- phone
  {
    name: "Charaktere",
    form: "phone",
    hash: "/chars",
    region: async (page) => {
      await expect(page.locator(".cc").first()).toBeVisible();
      await page.locator(".cc-open").first().click();
      await expect(page.locator(".cc-menu")).toBeVisible();
      return shown(page, "library", view(page));
    },
    skip: SKIP,
  },
  {
    name: "Charaktere: Auswahl",
    form: "phone",
    hash: "/chars",
    region: async (page) => {
      await page.locator(".title-row .actions .btn.icon-sm").click();
      await page.locator(".cc-check input").first().check();
      await expect(page.locator(".lib-selbar").getByRole("status")).toBeVisible();
      return shown(page, "library-select", view(page));
    },
    skip: SKIP,
  },
  {
    name: "Figur › Chats mit Menü",
    form: "phone",
    hash: "/chars",
    region: async (page) => {
      await go(page, `#/char/${await charId("Mara Voss")}/chats`);
      await expect(page.locator(".chat-row").first()).toBeVisible();
      await shown(page, "char-chats", charPage(page));
      await page.locator('.row-actions button[aria-haspopup="menu"]').first().click();
      await expect(page.locator(".row-menu")).toBeVisible();
      return shown(page, "char-chats-menu", page.locator(".chat-main, .row-menu"));
    },
    skip: SKIP,
  },
  {
    name: "Figur › Einstellungen",
    form: "phone",
    hash: "/chars",
    region: async (page) => {
      await go(page, `#/char/${await charId("Moos")}/settings`);
      await expect(page.locator('[data-spot="user-fit"] .uf-head')).toBeVisible();
      return shown(page, "char-settings", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Karte exportieren",
    form: "phone",
    hash: "/chars",
    region: async (page) => {
      await go(page, `#/char/${await charId("Mara Voss")}/settings`);
      await page.locator('.head-tools button[aria-haspopup="dialog"]').click();
      return shown(page, "card-export", page.locator("dialog.xf"));
    },
    skip: SKIP,
  },
  {
    name: "Personas",
    form: "phone",
    hash: "/personas",
    region: async (page) => {
      await expect(page.locator(".pl-row").first()).toBeVisible();
      return shown(page, "personas", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Persona bearbeiten",
    form: "phone",
    hash: "/personas",
    region: async (page) => {
      await go(page, `#/personas/${await personaId("Testperson Zwei")}`);
      await expect(page.locator(".pe-danger")).toBeVisible();
      return shown(page, "persona-edit", charPage(page));
    },
    skip: SKIP,
  },
  {
    name: "Persona eines Chats",
    form: "phone",
    hash: "/chars",
    region: async (page) => {
      await go(page, `#/chat/${await chatOf("Mara Voss")}`);
      await page.locator(".chat-persona").click();
      await expect(page.locator(".pm.sheet")).toBeVisible();
      return shown(page, "chat-persona", page.locator(".pm"));
    },
    skip: SKIP,
  },

  // ---- tablet (1280 × 800 with touch): the figure pane beside the cards
  {
    name: "Charaktere mit Figur",
    form: "tablet",
    hash: "/chars",
    region: async (page) => {
      await page.locator(".cc-open").first().click();
      await expect(page.locator(".tb-pane")).toBeVisible();
      await expect(page.locator(".tb-pane .tb-chat-link").first()).toBeVisible();
      return shown(page, "library-pane", view(page));
    },
    skip: SKIP,
  },
  {
    name: "Figur › Einstellungen",
    form: "tablet",
    hash: "/chars",
    region: async (page) => {
      await go(page, `#/char/${await charId("Moos")}/settings`);
      await expect(page.locator('[data-spot="user-fit"] .uf-head')).toBeVisible();
      return shown(page, "char-settings", charPage(page));
    },
    skip: SKIP,
  },
]);

// The list layout's test changes the library layout, a server setting: every test starts with the
// layout from before, and it is back after the file.
let layout = "poster";
const setLibraryLayout = (id: string) => api("PUT", "/settings", { library_layout: id });
test.beforeAll(async () => {
  layout = (await api<{ library_layout: string }>("GET", "/library")).library_layout;
});
test.beforeEach(() => setLibraryLayout(layout));
test.afterAll(() => setLibraryLayout(layout));
