// Chat comfort (K5): Escape stops an answer, arrow up in the empty field edits your last message,
// the unsent text of each chat survives a reload and a chat change, and an answer that is ready while
// the page is in the background marks the title, plays a tone and shows a notification (switchable).

import type { Page } from "@playwright/test";
import { charId, deleteChat, newChat } from "./api";
import { fillChat, longChatText } from "./db";
import { expect, test } from "./fixtures";

let chats: number[] = [];
test.afterEach(async () => {
  // A chat the model still writes in cannot be deleted yet: try again until the answer is done.
  for (const id of chats) {
    await expect(async () => {
      await deleteChat(id);
    }).toPass({ timeout: 120_000, intervals: [1000] });
  }
  chats = [];
});

async function freshChat(): Promise<number> {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  return id;
}

const box = (page: Page) => page.getByRole("textbox", { name: "Nachricht an Mara Voss" });
const mine = (page: Page) => page.getByRole("article", { name: "Deine Nachricht" });
const theirs = (page: Page) => page.getByRole("article", { name: "Nachricht der Figur" });
const sendButton = (page: Page) => page.getByRole("button", { name: "Senden" });
const stopButton = (page: Page) => page.getByRole("button", { name: "Antwort stoppen" });
/** The shortcut line under the field (drawn for a fine pointer; hidden by CSS on narrow screens). */
const hint = (page: Page) => page.locator(".composer .hint");
const editBox = (page: Page) => page.getByRole("textbox", { name: "Nachricht bearbeiten" });

async function send(page: Page, text: string) {
  await box(page).fill(text);
  await sendButton(page).click();
  await expect(mine(page).last()).toContainText(text);
}

/** Counts the tones started (WebAudio oscillators) and the system notifications shown. */
async function countSignals(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as { __tones: number; __notes: number; Notification?: typeof Notification };
    w.__tones = 0;
    w.__notes = 0;
    const osc = AudioContext.prototype.createOscillator;
    AudioContext.prototype.createOscillator = function (this: AudioContext) {
      w.__tones += 1;
      return osc.call(this);
    };
    if (w.Notification) {
      const N: typeof Notification = w.Notification;
      w.Notification = class extends N {
        static get permission() {
          return N.permission;
        }
        static requestPermission = N.requestPermission.bind(N);
        constructor(title: string, options?: NotificationOptions) {
          w.__notes += 1;
          super(title, options);
        }
      } as typeof Notification;
    }
  });
}

const signals = (page: Page) => page.evaluate(() => ({ tones: (window as unknown as { __tones: number }).__tones, notes: (window as unknown as { __notes: number }).__notes }));

/** The page as the browser reports it when the tab is in the background and the window has no focus. */
const goAway = (page: Page) =>
  page.evaluate(() => {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    Object.defineProperty(document, "hasFocus", { configurable: true, value: () => false });
    document.dispatchEvent(new Event("visibilitychange"));
  });

const comeBack = (page: Page) =>
  page.evaluate(() => {
    const d = document as unknown as Record<string, unknown>;
    delete d.hidden;
    delete d.visibilityState;
    delete d.hasFocus;
    document.dispatchEvent(new Event("visibilitychange"));
  });

test("Komfort: Esc stoppt die Antwort, gesendeter Text ist kein Entwurf mehr", async ({ page, open }) => {
  const id = await freshChat();
  await open(`/chat/${id}`);
  await send(page, "Erzähl mir bitte ausführlich und lange die ganze Geschichte deines Leuchtturms.");
  await expect(stopButton(page)).toBeVisible();
  // Text is streaming: then Escape stops it, wherever the focus is.
  await expect
    .poll(async () => (await theirs(page).nth(1).innerText().catch(() => "")).replace("Mara Voss", "").trim().length, { timeout: 60_000, intervals: [100] })
    .toBeGreaterThan(20);
  await expect(stopButton(page)).toBeVisible(); // still writing: what ends it is the key
  await expect(hint(page)).toContainText("Esc stoppt die Antwort");
  await page.keyboard.press("Escape");
  await expect(sendButton(page)).toBeVisible({ timeout: 15_000 });
  await expect(hint(page)).toContainText("letzte Nachricht bearbeiten");
  await expect(theirs(page)).toHaveCount(2); // the stopped answer stays

  // The sent text went with the message: after a reload the field is empty.
  await open(`/chat/${id}`);
  await expect(mine(page)).toHaveCount(1);
  await expect(box(page)).toHaveValue("");
});

test("Komfort: Pfeil hoch im leeren Feld bearbeitet die letzte eigene Nachricht", async ({ page, open }) => {
  const id = await freshChat();
  fillChat(id, 5); // greeting, yours (2), theirs (3), yours (4), theirs (5)
  await open(`/chat/${id}`);
  await expect(mine(page)).toHaveCount(2);

  // With text in the field, arrow up stays an ordinary key.
  await box(page).fill("noch nicht gesendet");
  await box(page).press("ArrowUp");
  await expect(editBox(page)).toHaveCount(0);

  await box(page).fill("");
  await box(page).press("ArrowUp");
  await expect(editBox(page)).toBeVisible();
  await expect(editBox(page)).toHaveValue(longChatText(4));
  await expect(editBox(page)).toBeFocused();
  // Escape leaves the edit, nothing else happens.
  await page.keyboard.press("Escape");
  await expect(editBox(page)).toHaveCount(0);
  await expect(mine(page).last()).toContainText("Nachricht 4:");
});

test("Komfort: Entwurf je Chat übersteht Neuladen und Chat-Wechsel", async ({ page, open }) => {
  const a = await freshChat();
  const b = await freshChat();
  await open(`/chat/${a}`);
  await box(page).fill("Entwurf für den ersten Chat");
  await page.reload();
  await expect(box(page)).toHaveValue("Entwurf für den ersten Chat");

  await page.evaluate((id) => (location.hash = `#/chat/${id}`), b);
  await expect(page).toHaveURL(new RegExp(`#/chat/${b}$`));
  await expect(box(page)).toHaveValue("");
  await box(page).fill("Zweiter Entwurf\nmit zwei Zeilen");

  await page.evaluate((id) => (location.hash = `#/chat/${id}`), a);
  await expect(box(page)).toHaveValue("Entwurf für den ersten Chat");
  await page.evaluate((id) => (location.hash = `#/chat/${id}`), b);
  await expect(box(page)).toHaveValue("Zweiter Entwurf\nmit zwei Zeilen");

  // Emptying the field forgets the draft.
  await box(page).fill("");
  await page.reload();
  await expect(box(page)).toHaveValue("");
});

test("Komfort: fertige Antwort im Hintergrund markiert den Titel, mit Ton und Mitteilung", async ({ page, open, context }) => {
  test.setTimeout(180_000);
  await context.grantPermissions(["notifications"]);
  await countSignals(page);
  const id = await freshChat();
  await open(`/chat/${id}`);
  await expect(page).toHaveTitle("Lorelamp");
  // Away before sending: a short answer from the cache can be ready within a moment.
  await goAway(page);
  await send(page, "Antworte nur mit einem Wort: Ja.");
  await expect(page).toHaveTitle("● Lorelamp", { timeout: 150_000 });
  expect(await signals(page)).toEqual({ tones: 2, notes: 1 });

  // Back on the page: the mark goes.
  await comeBack(page);
  await expect(page).toHaveTitle("Lorelamp");
});

test("Komfort: Schalter aus, dann kein Hinweis; über Strg K zu finden", async ({ page, open }) => {
  test.setTimeout(180_000);
  await countSignals(page);
  await open("/chars");
  await page.keyboard.press("Control+k");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  await dialog.getByRole("combobox").fill("benachrichtigung");
  await expect(dialog.getByRole("option").first()).toContainText("Hinweis, wenn eine Antwort fertig ist");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/settings$/);
  const main = page.getByRole("switch", { name: /Hinweis, wenn eine Antwort fertig ist/ });
  const tone = page.getByRole("switch", { name: /Mit leisem Ton/ });
  await expect(main).toBeFocused();
  await expect(main).toBeChecked();
  await expect(tone).toBeChecked();

  await main.click();
  await expect(main).not.toBeChecked();
  await expect(tone).toBeDisabled();
  await page.reload();
  await expect(page.getByRole("switch", { name: /Hinweis, wenn eine Antwort fertig ist/ })).not.toBeChecked();

  const id = await freshChat();
  await open(`/chat/${id}`);
  // Away before sending: a short answer from the cache can be ready within a moment.
  await goAway(page);
  await send(page, "Antworte nur mit einem Wort: Ja.");
  await expect(sendButton(page)).toBeVisible({ timeout: 150_000 });
  await expect(theirs(page).last().getByRole("button", { name: "Text kopieren" })).toBeVisible();
  await expect(page).toHaveTitle("Lorelamp");
  expect((await signals(page)).tones).toBe(0);
});
