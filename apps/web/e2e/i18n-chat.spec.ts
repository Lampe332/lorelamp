// The chat in English, Spanish and Russian holds no German (dictionary `chat`, see docs/i18n.md):
// the chat screen with its bar, messages, actions and composer, the search in the chat, editing a
// message, the large picture, the panel "Aussehen" with its backdrops, the tablet's figure beside the
// chat, and Settings → Chat with its sample and "Szene als Bild". Needs no model answer: it reads
// Mara's chat from the global setup (a message of yours and a measured reply).

import type { Page } from "@playwright/test";
import { api, charId } from "./api";
import { expect, test } from "./fixtures";
import { leftoverTests, type Screen } from "./leftovers";

/** Mara's chat from the global setup, read once before the screens open. */
let chatId = 0;
test.beforeAll(async () => {
  const { chats } = await api<{ chats: { id: number; messages: number }[] }>("GET", `/characters/${await charId("Mara Voss")}/chats`);
  chatId = (chats.find((c) => c.messages > 1) ?? chats[0]).id;
});

/** The user's own content: the messages and the card's text. */
const OWN = [".prose", "textarea", ".tb-figure-about"];

const chat = (page: Page) => page.locator(".chat-main");

/** Ctrl+F with a word that is in the chat, so the bar shows its count. */
async function find(page: Page) {
  await page.locator(".chat-main .composer textarea").focus();
  await page.keyboard.press("Control+f");
  const bar = page.locator(".find-bar");
  await expect(bar).toBeVisible();
  await page.keyboard.type("a");
  await expect(bar.locator(".find-count")).not.toBeEmpty();
  return bar;
}

/** Arrow up in the empty field opens your last message for editing. */
async function editing(page: Page) {
  const field = page.locator(".chat-main .composer textarea");
  await field.fill("");
  await field.focus();
  await page.keyboard.press("ArrowUp");
  const box = page.locator(".msg-edit");
  await expect(box).toBeVisible();
  return box;
}

/** The panel "Aussehen": the button in the bar (desktop, tablet) or in the message field while
 *  nothing is typed there (phone). */
async function look(page: Page) {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  const button = page.locator(".cv-look-btn:visible, .cv-look-box:visible").first();
  await button.click();
  const panel = page.locator(".cv-panel");
  await expect(panel).toBeVisible();
  return panel;
}

/** The large picture beside the story (desktop): the character's picture in the bar switches it on. */
async function stage(page: Page) {
  await page.locator(".chat-bar .cv-av").click();
  const pic = page.locator(".cv-stage");
  await expect(pic).toBeVisible();
  return page.locator(".chat-bar, .cv-stage");
}

/** A screen in Mara's chat; its address is read when the test opens it (the id is known by then). */
function inChat(name: string, form: Screen["form"], region: Screen["region"], skip?: string[]): Screen {
  return {
    name,
    form,
    get hash() {
      return `/chat/${chatId}`;
    },
    region,
    skip,
  };
}

const screens = (form: Screen["form"]): Screen[] => [
  inChat("Chat", form, chat, OWN),
  inChat("Im Chat suchen", form, find),
  inChat("Nachricht bearbeiten", form, editing, OWN),
  inChat("Aussehen", form, look),
  { name: "Einstellungen › Chat", form, hash: "/settings/chat", region: (page) => page.locator(".cv-tab") },
];

leftoverTests("chat", [
  ...screens("desktop"),
  inChat("Großes Bild", "desktop", stage, OWN),
  ...screens("phone"),
  // The tablet shows the figure with its chats beside the story.
  ...screens("tablet"),
]);
