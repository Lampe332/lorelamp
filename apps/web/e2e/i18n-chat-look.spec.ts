// The wider choice of how a chat looks in English, Spanish and Russian holds no German (dictionary
// `chat`): Settings → Chat and a chat's "Aussehen" with "Messenger breit" chosen (so its own fields
// show), times and "Punkte und Name" on, and the chat itself drawn that way. Needs no model answer: it
// reads Mara's chat from the global setup.

import type { Page } from "@playwright/test";
import { api, charId } from "./api";
import { expect, test } from "./fixtures";
import { leftoverTests, type Screen } from "./leftovers";

let chatId = 0;
test.beforeAll(async () => {
  const { chats } = await api<{ chats: { id: number; messages: number }[] }>("GET", `/characters/${await charId("Mara Voss")}/chats`);
  chatId = (chats.find((c) => c.messages > 1) ?? chats[0]).id;
  await api("PUT", "/chat-look", { style: "wide", times: true, typing: "name", av_shape: "soft" });
});
test.afterAll(() => api("PUT", "/chat-look", {}));

/** The user's own content: the messages and the card's text. */
const OWN = [".prose", "textarea"];

async function look(page: Page) {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.locator(".cv-look-btn:visible, .cv-look-box:visible").first().click();
  const panel = page.locator(".cv-panel");
  await expect(panel).toBeVisible();
  return panel.locator(".cv-panel-body");
}

const inChat = (name: string, form: Screen["form"], region: Screen["region"], skip?: string[]): Screen => ({
  name,
  form,
  get hash() {
    return `/chat/${chatId}`;
  },
  region,
  skip,
});

const screens = (form: Screen["form"]): Screen[] => [
  inChat("Chat, Messenger breit", form, (page) => page.locator(".chat-main"), OWN),
  inChat("Aussehen, Messenger breit", form, look),
  { name: "Einstellungen › Chat, Messenger breit", form, hash: "/settings/chat", region: (page) => page.locator(".cv-tab") },
];

leftoverTests("chat-look", [...screens("desktop"), ...screens("phone")]);
