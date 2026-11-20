// Settings tab "Allgemein" (Q2): each setting is changed on the page and then shown to work.
// What the app opens with, the key that sends, asking before a delete and before a rewind, how long
// "Rückgängig" stays, the automatic backup, starting with Windows and the data folder; Ctrl+K finds
// each of them and lights up its place.
//
// Needs `RP_E2E_DATA` (chats are filled straight into the test database; the data folder's path).
// Starting with Windows writes into the registry: the test server must be started with
// `RP_AUTOSTART_KEY` set to a scratch key below HKEY_CURRENT_USER and the tests with the same key
// in `RP_E2E_AUTOSTART_KEY` (never the real `Run` key); without it that test skips itself.
// Sending a real message needs a model that answers; without one that test skips itself and the
// key test beside it still runs. `RP_E2E_SHOTS` (optional): review screenshots go there.

import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { api, charId, deleteChat, newChat, token } from "./api";
import { dataDir, fillChat } from "./db";
import { expect, isPhone, test } from "./fixtures";

type General = {
  send_key: string;
  start_page: string;
  confirm_delete: boolean;
  confirm_rewind: boolean;
  undo_secs: number;
  backup_rhythm: string;
  backup_next_ms: number | null;
  data_dir: string | null;
  autostart: { on: boolean; minimized: boolean } | null;
};

const general = () => api<General>("GET", "/general");
const DEFAULTS = { send_key: "enter", start_page: "home", confirm_delete: false, confirm_rewind: false, undo_secs: 8, backup_rhythm: "daily" };
const autostartKey = process.env.RP_E2E_AUTOSTART_KEY ?? "";
const shots = process.env.RP_E2E_SHOTS;
const DAY = 86_400_000;

let chats: number[] = [];
test.afterEach(async () => {
  await api("PUT", "/general", DEFAULTS);
  // A chat the model still writes in refuses its delete (409); wait for the answer to end, so no
  // test chat stays behind for the later specs (chat counts, the start screen's recent chats).
  for (const id of chats) {
    await expect
      .poll(() => deleteChat(id).then(() => "gone", (e: Error) => (/ 409 /.test(e.message) ? "busy" : "gone")), { timeout: 60_000 })
      .toBe("gone");
  }
  chats = [];
});

/** A new chat with Mara Voss, with a title of its own so its row can be found. */
async function chat(title: string, messages = 0): Promise<number> {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  await api("PUT", `/chats/${id}`, { title });
  if (messages) fillChat(id, messages);
  return id;
}

const chatCount = async () => (await api<{ chats: unknown[] }>("GET", `/characters/${await charId("Mara Voss")}/chats`)).chats.length;
const field = (page: Page) => page.getByRole("textbox", { name: "Nachricht an Mara Voss" });
const ask = (page: Page) => page.getByRole("dialog").filter({ has: page.getByRole("button", { name: "Abbrechen" }) });
const toast = (page: Page) => page.locator(".toast");
const mine = (page: Page, text: string) => page.getByRole("article", { name: "Deine Nachricht" }).filter({ hasText: text });
const modelAnswers = async () => (await api<{ status: string }>("GET", "/engine")).status === "ready";

/** Stops the answer the model started (it may also be done already) and waits until the chat is free,
 *  so the next send is not refused because the model still writes here. */
async function stopAnswer(page: Page) {
  const stop = page.getByRole("button", { name: "Antwort stoppen" });
  await stop.click({ timeout: 15_000 }).catch(() => {});
  await expect(stop).toHaveCount(0, { timeout: 60_000 });
}

test("Allgemein: die neuen Bereiche stehen da, jede Wahl wird gespeichert", async ({ page, open }) => {
  await open("/settings");
  for (const name of ["Start", "Schreiben", "Nachfragen", "Hinweise", "Daten"]) {
    await expect(page.getByRole("heading", { name, level: 2, exact: true })).toBeVisible();
  }
  await page.getByRole("combobox", { name: "Beim Start öffnen" }).selectOption({ label: "Charaktere" });
  // The send key is a choice at a desk only; a phone names no keys and sends with its button.
  const desk = !isPhone(page);
  if (desk) {
    // The radio itself is hidden behind its card: the card is what gets pressed.
    await page.locator("label.choice").filter({ hasText: "Strg" }).click();
  } else {
    await expect(page.getByRole("radiogroup", { name: "Senden mit" })).toHaveCount(0);
    await expect(page.locator('[data-spot="send-key"]')).toContainText("Hier schickt der Knopf neben dem Feld die Nachricht ab.");
  }
  await page.getByRole("switch", { name: /^Vor dem Löschen nachfragen/ }).check();
  await page.getByRole("combobox", { name: /Rückgängig.*bleibt stehen/ }).selectOption({ label: "30 Sekunden" });
  const key = desk ? "ctrl_enter" : "enter";
  await expect.poll(general).toMatchObject({ start_page: "characters", send_key: key, confirm_delete: true, confirm_rewind: false, undo_secs: 30 });
  // The choice holds after a reload (it lives on the server, not in the page).
  await page.reload();
  await expect(page.getByRole("combobox", { name: "Beim Start öffnen" })).toHaveValue("characters");
  if (desk) await expect(page.getByRole("radio", { name: /^Strg \+ Enter/ })).toBeChecked();
  await expect(page.getByRole("switch", { name: /^Vor dem Löschen nachfragen/ })).toBeChecked();
  // The server refuses values it does not know and stores nothing of such a change.
  await expect(api("PUT", "/general", { send_key: desk ? "enter" : "ctrl_enter", undo_secs: 7 })).rejects.toThrow(/400.*bad_undo_secs/);
  expect((await general()).send_key).toBe(key);
});

test("Allgemein: eine Änderung kommt auf einem zweiten Gerät ohne Neuladen an", async ({ page, open }) => {
  await open("/settings");
  const rewind = page.getByRole("switch", { name: /^Vor dem Zurückspulen nachfragen/ });
  await expect(rewind).not.toBeChecked();
  await api("PUT", "/general", { confirm_rewind: true }); // as another device would
  await expect(rewind).toBeChecked();
});

test("Beim Start öffnen: Charaktere, der zuletzt benutzte Chat, sonst die Startseite", async ({ page, open }) => {
  const last = await chat("Zuletzt benutzt", 4);
  await open("/settings");
  const pick = page.getByRole("combobox", { name: "Beim Start öffnen" });
  const start = `/?token=${token}`;

  await pick.selectOption({ label: "Charaktere" });
  await expect.poll(async () => (await general()).start_page).toBe("characters");
  await page.goto(start);
  await expect(page).toHaveURL(/#\/chars$/);
  // Going back to "Start" in the same window stays there: the choice is for opening the app.
  await page.goto(`${start}#/`);
  await page.reload();
  await expect(page.getByRole("region", { name: "Weiterlesen" }).or(page.getByRole("heading", { level: 1 })).first()).toBeVisible();
  await expect(page).not.toHaveURL(/#\/chars$/);

  // The chat written in last: the one the library lists first.
  await api("PUT", "/general", { start_page: "last_chat" });
  expect((await api<{ recent: { chat: { id: number } }[] }>("GET", "/library")).recent[0].chat.id).toBe(last);
  await page.evaluate(() => sessionStorage.clear()); // a newly opened window
  await page.goto("about:blank");
  await page.goto(start);
  await expect(page).toHaveURL(new RegExp(`#/chat/${last}$`));
  await expect(field(page)).toBeVisible();

  // The start screen again.
  await api("PUT", "/general", { start_page: "home" });
  await page.evaluate(() => sessionStorage.clear());
  await page.goto("about:blank");
  await page.goto(start);
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  await expect(page).not.toHaveURL(/#\/(chars|chat)/);
});

test("Senden mit: Enter macht je nach Wahl eine neue Zeile oder nicht", async ({ page, open }) => {
  const answers = await modelAnswers();
  const id = await chat("Tasten");
  await open(`/chat/${id}`);
  await field(page).fill("Erste Zeile");
  // Enter sends, so it never breaks the line. With a model that answers the message goes out and its
  // answer is stopped, so the model is free again for the sends below and for the next tests.
  await field(page).press("Enter");
  await expect(field(page)).not.toHaveValue(/\n/);
  if (answers) {
    await expect(mine(page, "Erste Zeile")).toHaveCount(1);
    await stopAnswer(page);
  }
  if (!isPhone(page)) await expect(page.locator(".composer .hint")).toContainText("Enter sendet");

  if (isPhone(page)) {
    // A phone shows no key choice (it sends with its button): the key is chosen at the PC.
    await api("PUT", "/general", { send_key: "ctrl_enter" });
  } else {
    await open("/settings");
    // The radio itself is hidden behind its card: the card is what gets pressed.
    await page.locator("label.choice").filter({ hasText: "Strg" }).click();
  }
  await expect.poll(async () => (await general()).send_key).toBe("ctrl_enter");
  await open(`/chat/${id}`);
  await field(page).fill("Erste Zeile");
  await field(page).press("Enter");
  await field(page).pressSequentially("Zweite Zeile");
  await expect(field(page)).toHaveValue("Erste Zeile\nZweite Zeile");
  // Ctrl + Enter is the sending key now: no line break from it.
  await field(page).press("Control+Enter");
  await expect(field(page)).not.toHaveValue(/Zweite Zeile\n/);
  if (!isPhone(page)) await expect(page.locator(".composer .hint")).toContainText("Strg + Enter sendet");
  if (answers) {
    await expect(mine(page, "Zweite Zeile")).toHaveCount(1);
    await stopAnswer(page);
  }
});

test("Senden mit Strg + Enter: Enter bricht um, Strg + Enter schickt die Nachricht ab", async ({ page, open }) => {
  test.skip(!(await modelAnswers()), "needs a model that answers");
  const id = await chat("Senden");
  await api("PUT", "/general", { send_key: "ctrl_enter" });
  await open(`/chat/${id}`);
  await field(page).fill("Zeile eins");
  await field(page).press("Enter");
  await field(page).pressSequentially("Zeile zwei");
  await expect(page.getByRole("article").filter({ hasText: "Zeile eins" })).toHaveCount(0);
  await field(page).press("Control+Enter");
  await expect(page.getByRole("article").filter({ hasText: "Zeile eins" }).filter({ hasText: "Zeile zwei" })).toHaveCount(1);
  await expect(field(page)).toHaveValue("");
  await stopAnswer(page);
});

test("Vor dem Löschen nachfragen: erst die Frage, Abbrechen behält den Chat", async ({ page, open }) => {
  await chat("Löschprobe Eins");
  await chat("Löschprobe Zwei");
  const before = await chatCount();
  await open("/settings");
  await page.getByRole("switch", { name: /^Vor dem Löschen nachfragen/ }).check();
  await expect.poll(async () => (await general()).confirm_delete).toBe(true);

  await open(`/char/${await charId("Mara Voss")}/chats`);
  await page.getByRole("button", { name: "Löschprobe Eins löschen" }).click();
  await expect(ask(page)).toBeVisible();
  await expect(ask(page).getByRole("heading")).toHaveText("„Löschprobe Eins“ löschen?");
  await ask(page).getByRole("button", { name: "Abbrechen" }).click();
  await expect(ask(page)).toHaveCount(0);
  expect(await chatCount()).toBe(before);
  await expect(toast(page)).toHaveCount(0);

  await page.getByRole("button", { name: "Löschprobe Eins löschen" }).click();
  await ask(page).getByRole("button", { name: "Löschen", exact: true }).click();
  await expect(toast(page)).toContainText("„Löschprobe Eins“ gelöscht");
  await expect.poll(chatCount).toBe(before - 1);

  // Switched off again: the delete runs at once, "Rückgängig" is the safety net.
  await api("PUT", "/general", { confirm_delete: false });
  await expect(page.getByRole("button", { name: "Löschprobe Zwei löschen" })).toBeVisible();
  await page.getByRole("button", { name: "Löschprobe Zwei löschen" }).click();
  await expect(toast(page)).toContainText("„Löschprobe Zwei“ gelöscht");
  await expect(ask(page)).toHaveCount(0);
  await expect.poll(chatCount).toBe(before - 2);
});

test("Vor dem Zurückspulen nachfragen: erst die Frage, Abbrechen behält die Nachrichten", async ({ page, open }) => {
  const id = await chat("Spulprobe", 6);
  await open("/settings");
  await page.getByRole("switch", { name: /^Vor dem Zurückspulen nachfragen/ }).check();
  await expect.poll(async () => (await general()).confirm_rewind).toBe(true);

  await open(`/chat/${id}`);
  const messages = page.getByRole("article");
  await expect(messages).toHaveCount(6);
  const fifth = messages.filter({ hasText: "Nachricht 5:" });
  const rewind = async () => {
    await fifth.hover();
    await fifth.getByRole("button", { name: "Bis hier zurückspulen" }).click();
  };
  await rewind();
  await expect(ask(page).getByRole("heading")).toHaveText("Bis hierher zurückspulen?");
  await page.keyboard.press("Escape");
  await expect(ask(page)).toHaveCount(0);
  await expect(messages).toHaveCount(6);

  await rewind();
  await ask(page).getByRole("button", { name: "Zurückspulen", exact: true }).click();
  await expect(messages).toHaveCount(4);
  await expect(toast(page)).toContainText("Zurückgespult: 2 Nachrichten entfernt");

  // Switched off: no question.
  await api("PUT", "/general", { confirm_rewind: false });
  const third = messages.filter({ hasText: "Nachricht 3:" });
  await third.hover();
  await third.getByRole("button", { name: "Bis hier zurückspulen" }).click();
  await expect(messages).toHaveCount(2);
  await expect(ask(page)).toHaveCount(0);
});

test("„Rückgängig“ bleibt so lange stehen, wie gewählt", async ({ page, open }) => {
  await chat("Kurz");
  await chat("Lang");
  await open("/settings");
  const pick = page.getByRole("combobox", { name: /Rückgängig.*bleibt stehen/ });
  await pick.selectOption({ label: "5 Sekunden" });
  await expect.poll(async () => (await general()).undo_secs).toBe(5);

  await open(`/char/${await charId("Mara Voss")}/chats`);
  const undo = toast(page).getByRole("button", { name: "Rückgängig" });
  await page.getByRole("button", { name: "Kurz löschen" }).click();
  await expect(undo).toBeVisible();
  let shown = Date.now();
  await expect(undo).toHaveCount(0, { timeout: 8000 });
  expect(Date.now() - shown).toBeGreaterThan(4000);
  expect(Date.now() - shown).toBeLessThan(7000);

  // 15 seconds: still there when the usual 8 are long over, and it still works.
  await api("PUT", "/general", { undo_secs: 15 });
  await page.getByRole("button", { name: "Lang löschen" }).click();
  await expect(undo).toBeVisible();
  shown = Date.now();
  await page.waitForTimeout(10_500);
  await expect(undo).toBeVisible();
  await undo.click();
  await expect(page.getByRole("button", { name: "Lang löschen" })).toBeVisible();
  expect(Date.now() - shown).toBeLessThan(15_000);
});

test("Automatische Sicherung: täglich, wöchentlich oder aus, mit dem nächsten Termin", async ({ page, open }) => {
  await open("/settings");
  const pick = page.getByRole("combobox", { name: /^Automatische Sicherung/ });
  const next = page.locator('[data-spot="backup-rhythm"]').getByRole("status");
  await expect(pick).toHaveValue("daily");
  // The server made its daily backup at its start, so the next one is a day after it.
  const daily = (await general()).backup_next_ms!;
  expect(daily - Date.now()).toBeLessThanOrEqual(DAY);
  await expect(next).toContainText(/^Nächste Sicherung: (heute|morgen|in den nächsten Minuten)/);

  await pick.selectOption({ label: "Wöchentlich" });
  await expect.poll(async () => (await general()).backup_rhythm).toBe("weekly");
  // Six days later than with "täglich": a week after the newest automatic backup.
  expect((await general()).backup_next_ms).toBe(daily + 6 * DAY);
  await expect(next).toContainText(/^Nächste Sicherung: am \w+, \d+\. \w+ ab \d\d:\d\d Uhr/);

  await pick.selectOption({ label: "Aus" });
  await expect.poll(async () => (await general()).backup_rhythm).toBe("off");
  expect((await general()).backup_next_ms).toBeNull();
  await expect(next).toContainText("Lorelamp sichert nicht mehr von selbst.");
  // The backups themselves are one click away.
  await page.getByRole("link", { name: "Sicherungen ansehen" }).click();
  await expect(page).toHaveURL(/#\/settings\/backups$/);
});

/** The autostart entry in the scratch key, as Windows would read it ("" when there is none). */
function runEntry(): string {
  try {
    const out = execFileSync("reg", ["query", `HKCU\\${autostartKey}`, "/v", "Lorelamp"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
    return /REG_SZ\s+(.*)/.exec(out)?.[1].trim() ?? "";
  } catch {
    return "";
  }
}

test("Mit Windows starten: der Schalter schreibt den Autostart-Eintrag, minimiert hängt --minimized an", async ({ page, open }) => {
  test.skip(!autostartKey, "needs RP_E2E_AUTOSTART_KEY (the scratch key the test server was started with)");
  test.skip(/\\CurrentVersion\\Run$/i.test(autostartKey), "never the real Run key");
  await api("PUT", "/general/autostart", { on: false, minimized: false });
  expect(runEntry()).toBe("");
  await open("/settings");
  const on = page.getByRole("switch", { name: /^Mit Windows starten/ });
  const minimized = page.getByRole("switch", { name: /^Dabei minimiert starten/ });
  await expect(on).not.toBeChecked();
  await expect(minimized).toBeDisabled();

  await on.check();
  await expect.poll(runEntry).toMatch(/^"[^"]+rp-server\.exe"$/i);
  await expect(minimized).toBeEnabled();
  await minimized.check();
  await expect.poll(runEntry).toMatch(/^"[^"]+rp-server\.exe" --minimized$/i);
  expect((await general()).autostart).toEqual({ on: true, minimized: true });
  // The registry is the truth: after a reload the page shows what is written there.
  await page.reload();
  await expect(on).toBeChecked();
  await expect(minimized).toBeChecked();

  await on.uncheck();
  await expect.poll(runEntry).toBe("");
  await expect(minimized).toBeDisabled();
  await expect(minimized).not.toBeChecked();
});

/** Explorer windows that show `dir`, closed again; answers how many there were. */
function closeExplorerOn(dir: string): number {
  const script = `$n = 0; foreach ($w in @((New-Object -ComObject Shell.Application).Windows())) { try { if ($w.Document.Folder.Self.Path -ieq $env:RP_DIR) { $n++; $w.Quit() } } catch {} }; $n`;
  const out = execFileSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8", env: { ...process.env, RP_DIR: dir } });
  return Number(out.trim()) || 0;
}

test("Datenordner: der Pfad steht da, der Knopf öffnet ihn im Explorer", async ({ page, open }, info) => {
  test.skip(!dataDir, "needs RP_E2E_DATA");
  await open("/settings");
  const row = page.locator('[data-spot="data-folder"]');
  await expect(row.locator("code")).toHaveText(dataDir);
  const button = row.getByRole("button", { name: "Im Explorer öffnen" });
  await expect(button).toBeEnabled();
  // One Explorer window on the owner's screen is enough: only one project really opens it.
  test.skip(info.project.name !== "desktop-graphit", "opens a real Explorer window; once is enough");
  closeExplorerOn(dataDir);
  await button.click();
  await expect(row.getByRole("status")).toHaveText("Der Ordner ist im Explorer offen.");
  await expect.poll(() => closeExplorerOn(dataDir), { timeout: 15_000 }).toBeGreaterThan(0);
});

const PLACES: [query: string, label: string, spot: string][] = [
  ["beim start öffnen", "Beim Start öffnen", "start-page"],
  ["autostart", "Mit Windows starten", "autostart"],
  ["minimiert", "Minimiert starten", "start-minimized"],
  ["senden mit enter", "Senden mit Enter oder Strg + Enter", "send-key"],
  ["löschen nachfragen", "Vor dem Löschen nachfragen", "confirm-delete"],
  ["zurückspulen nachfragen", "Vor dem Zurückspulen nachfragen", "confirm-rewind"],
  ["rückgängig zeit", "Wie lange „Rückgängig“ bleibt", "undo-time"],
  ["automatische sicherung", "Automatische Sicherung", "backup-rhythm"],
  ["datenordner", "Datenordner öffnen", "data-folder"],
];

test("Strg K findet jede Einstellung und zeigt genau ihre Stelle", async ({ page, open }) => {
  const scratch = (await general()).autostart !== null;
  await open("/chars");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  for (const [query, label, spot] of PLACES) {
    await page.keyboard.press("Control+k");
    await dialog.getByRole("combobox").fill(query);
    await expect(dialog.getByRole("option").first()).toContainText(label);
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect(page).toHaveURL(/#\/settings$/);
    // Where the app cannot start with Windows, both entries point at the note that says so.
    const place = page.locator(`[data-spot="${scratch || !spot.startsWith("start-min") ? spot : "autostart"}"]`);
    await expect(place).toHaveClass(/spot-on/);
    await expect(place).toBeInViewport();
    // Leave the page, so the next entry has to open it again.
    await page.goto(page.url().replace(/#.*$/, "#/chars"));
  }
});

test("Allgemein: Bildschirmfotos zur Durchsicht", async ({ page, open, theme }, info) => {
  test.skip(!shots, "review screenshots only with RP_E2E_SHOTS");
  mkdirSync(shots!, { recursive: true });
  const size = isPhone(page) ? "375" : "1920";
  await open("/settings");
  await expect(page.getByRole("combobox", { name: "Beim Start öffnen" })).toBeEnabled();
  // The whole page, however long: the scrolling part is given its full height for the picture.
  const full = async (name: string) => {
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: join(shots!, `${name}-${theme}.png`), fullPage: true });
  };
  await full(`general-${size}`);
  for (const spot of ["start-page", "send-key", "confirm-delete", "backup-rhythm"]) {
    await page.locator(`[data-spot="${spot}"]`).scrollIntoViewIfNeeded();
    await page.screenshot({ path: join(shots!, `general-${size}-${spot}-${theme}.png`) });
  }
  // The question before a delete.
  await api("PUT", "/general", { confirm_delete: true });
  await chat("Bildprobe");
  await open(`/char/${await charId("Mara Voss")}/chats`);
  await page.getByRole("button", { name: "Bildprobe löschen" }).click();
  await expect(ask(page)).toBeVisible();
  await page.screenshot({ path: join(shots!, `general-${size}-ask-${theme}.png`), animations: "disabled" });
  await ask(page).getByRole("button", { name: "Abbrechen" }).click();
  expect(info.errors).toHaveLength(0);
});

test.describe("Allgemein auf dem Tablet", () => {
  test.use({ viewport: { width: 1024, height: 1366 }, hasTouch: true, isMobile: true });
  test("Bildschirmfotos und Bedienung mit dem Finger", async ({ page, open, theme }, info) => {
    test.skip(info.project.name.startsWith("phone-"), "own window size; one run per theme is enough");
    await open("/settings");
    await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
    const pick = page.getByRole("combobox", { name: "Beim Start öffnen" });
    await expect(pick).toBeEnabled();
    // Every control of the new rows is at least 44 px high for a finger.
    for (const spot of ["start-page", "undo-time", "backup-rhythm"]) {
      const box = await page.locator(`[data-spot="${spot}"] select`).boundingBox();
      expect(box!.height, spot).toBeGreaterThanOrEqual(44);
    }
    await page.getByRole("switch", { name: /^Vor dem Löschen nachfragen/ }).tap();
    await expect.poll(async () => (await general()).confirm_delete).toBe(true);
    if (!shots) return;
    mkdirSync(shots, { recursive: true });
    for (const spot of ["start-page", "confirm-delete", "data-folder"]) {
      await page.locator(`[data-spot="${spot}"]`).scrollIntoViewIfNeeded();
      await page.screenshot({ path: join(shots, `general-tablet-${spot}-${theme}.png`) });
    }
  });
});
