// Chat export on the character page: one chat through its row menu and the format dialog (Markdown
// and SillyTavern), several through the selection mode as one ZIP, and Ctrl+K "Chat exportieren".
// The chats get their messages straight into the test database (`e2e/db.ts`), no model needed.

import { readFile } from "node:fs/promises";
import { inflateRawSync } from "node:zlib";
import type { Download, Page } from "@playwright/test";
import { api, charId, deleteChat, library, newChat } from "./api";
import { fillChat, longChatText } from "./db";
import { expect, test } from "./fixtures";
import { withoutSavePicker } from "./save-picker";

// The plain download (as on a phone); the save dialog has its own tests in save-dialog.spec.ts.
test.beforeEach(({ page }) => withoutSavePicker(page));

const MESSAGES = 6;

let chats: number[] = [];
test.afterEach(async () => {
  for (const id of chats) await deleteChat(id);
  chats = [];
});

/** A new chat with Mara: the greeting, then yours and hers in turn (`longChatText`), named `title`. */
async function chatWith(title: string): Promise<number> {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  fillChat(id, MESSAGES);
  await api("PUT", `/chats/${id}`, { title });
  return id;
}

/** Who the user plays as: the chats here follow the active persona. */
async function personaName(): Promise<string> {
  const lib = await library();
  return lib.personas.find((p) => p.id === lib.active_persona_id)?.name ?? "";
}

/** Every API address the page asks for, to check that the token never sits in one. */
function watchExports(page: Page): string[] {
  const urls: string[] = [];
  page.on("request", (r) => {
    if (r.url().includes("/export")) urls.push(r.url());
  });
  return urls;
}

async function save(page: Page, click: () => Promise<void>): Promise<{ download: Download; bytes: Buffer }> {
  const [download] = await Promise.all([page.waitForEvent("download"), click()]);
  const path = await download.path();
  return { download, bytes: await readFile(path) };
}

/** The files in a ZIP (stored or deflated), read from its central directory. */
function unzip(zip: Buffer): Map<string, string> {
  let end = zip.length - 22;
  while (end >= 0 && zip.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < 0) throw new Error("not a zip");
  const count = zip.readUInt16LE(end + 10);
  let at = zip.readUInt32LE(end + 16);
  const files = new Map<string, string>();
  for (let i = 0; i < count; i++) {
    if (zip.readUInt32LE(at) !== 0x02014b50) throw new Error("broken central directory");
    const method = zip.readUInt16LE(at + 10);
    const size = zip.readUInt32LE(at + 20);
    const nameLen = zip.readUInt16LE(at + 28);
    const extraLen = zip.readUInt16LE(at + 30);
    const commentLen = zip.readUInt16LE(at + 32);
    const local = zip.readUInt32LE(at + 42);
    const name = zip.subarray(at + 46, at + 46 + nameLen).toString("utf8");
    const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    const data = zip.subarray(start, start + size);
    files.set(name, (method === 8 ? inflateRawSync(data) : data).toString("utf8"));
    at += 46 + nameLen + extraLen + commentLen;
  }
  return files;
}

test("Export: ein Chat als Markdown und als SillyTavern-Datei", async ({ page, open }) => {
  await chatWith("Exporttest");
  const user = await personaName();
  const urls = watchExports(page);
  await open(`/char/${await charId("Mara Voss")}/chats`);

  const menuButton = page.getByRole("button", { name: "Weitere Aktionen für Exporttest" });
  const exportAs = async (format: RegExp) => {
    await menuButton.click();
    await page.getByRole("menu", { name: "Aktionen für Exporttest" }).getByRole("menuitem", { name: /^Exportieren/ }).click();
    const dialog = page.getByRole("dialog", { name: "Chat exportieren" });
    await expect(dialog).toBeVisible();
    await dialog.getByRole("radio", { name: format }).check();
    const file = await save(page, () => dialog.getByRole("button", { name: "Exportieren", exact: true }).click());
    await expect(dialog).toHaveCount(0);
    return file;
  };

  const md = await exportAs(/^Markdown/);
  expect(md.download.suggestedFilename()).toMatch(/^Mara Voss - Exporttest - \d{4}-\d{2}-\d{2}-\d{4}\.md$/);
  const text = md.bytes.toString("utf8");
  expect(text.startsWith("# Mara Voss · Exporttest\n")).toBe(true);
  expect(text).toContain(`- **Persona:** ${user}\n`);
  expect(text.match(/\n\*\*Mara Voss\*\*\n/g)).toHaveLength(MESSAGES / 2);
  expect(text.match(new RegExp(`\\n\\*\\*${user}\\*\\*\\n`, "g"))).toHaveLength(MESSAGES / 2);
  for (let n = 2; n <= MESSAGES; n++) expect(text).toContain(longChatText(n));

  const st = await exportAs(/^SillyTavern/);
  expect(st.download.suggestedFilename()).toMatch(/^Mara Voss - Exporttest - .*\.jsonl$/);
  const [head, ...lines] = st.bytes.toString("utf8").split("\n");
  expect(JSON.parse(head)).toMatchObject({ user_name: user, character_name: "Mara Voss", chat_metadata: {} });
  const messages = lines.map((l) => JSON.parse(l) as { name: string; is_user: boolean; is_system: boolean; mes: string; send_date: string });
  expect(messages).toHaveLength(MESSAGES);
  messages.forEach((m, i) => {
    const n = i + 1;
    expect(m.is_user).toBe(n % 2 === 0);
    expect(m.name).toBe(m.is_user ? user : "Mara Voss");
    expect(m.is_system).toBe(false);
    expect(new Date(m.send_date).getTime()).toBeGreaterThan(0);
    if (n > 1) expect(m.mes).toBe(longChatText(n));
  });

  // The last choice is remembered for the next export.
  await menuButton.click();
  await page.getByRole("menuitem", { name: /^Exportieren/ }).click();
  await expect(page.getByRole("dialog", { name: "Chat exportieren" }).getByRole("radio", { name: /^SillyTavern/ })).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);

  expect(urls.length).toBeGreaterThanOrEqual(2);
  for (const url of urls) expect(url).not.toContain("token=");
});

test("Export: mehrere Chats als ZIP über „Auswählen“", async ({ page, open }) => {
  await chatWith("Export A");
  await chatWith("Export B");
  const urls = watchExports(page);
  await open(`/char/${await charId("Mara Voss")}/chats`);

  await page.getByRole("button", { name: "Auswählen", exact: true }).click();
  const zipButton = page.getByRole("toolbar", { name: "Auswahl" }).getByRole("button", { name: "Exportieren (ZIP)" });
  await expect(zipButton).toBeDisabled();
  await page.getByRole("checkbox", { name: "Export A auswählen" }).check();
  await page.getByRole("checkbox", { name: "Export B auswählen" }).check();
  await expect(page.getByRole("toolbar", { name: "Auswahl" }).getByRole("status")).toContainText("2 von");
  await zipButton.click();

  const dialog = page.getByRole("dialog", { name: "2 Chats exportieren" });
  await dialog.getByRole("radio", { name: /^Text/ }).check();
  const zip = await save(page, () => dialog.getByRole("button", { name: "Exportieren (ZIP)" }).click());
  expect(zip.download.suggestedFilename()).toMatch(/^Chats - \d{4}-\d{2}-\d{2}-\d{4}\.zip$/);
  const files = unzip(zip.bytes);
  const names = [...files.keys()].sort();
  expect(names).toHaveLength(2);
  expect(names[0]).toMatch(/^Mara Voss - Export A - .*\.txt$/);
  expect(names[1]).toMatch(/^Mara Voss - Export B - .*\.txt$/);
  for (const text of files.values()) {
    expect(text).toContain("Figur: Mara Voss\n");
    expect(text).toContain(longChatText(MESSAGES));
    expect(text).not.toContain("**");
  }
  // Done: the selection mode ends.
  await expect(page.getByRole("toolbar", { name: "Auswahl" })).toHaveCount(0);
  for (const url of urls) expect(url).not.toContain("token=");
});

test("Strg K: „Chat exportieren“ öffnet die Chats der Figur mit Hinweis", async ({ page, open }) => {
  await open("/chars");
  await page.keyboard.press("Control+k");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  await dialog.getByRole("combobox").fill("chat exportieren");
  await expect(dialog.getByRole("option").first()).toContainText("Chat exportieren");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/char\/\d+\/chats$/);
  const hint = page.getByRole("note");
  await expect(hint).toContainText("Exportieren (ZIP)");
  await expect(page.locator(".spot-on")).toBeInViewport();
  await hint.getByRole("button", { name: "Hinweis schließen" }).click();
  await expect(page.getByRole("note")).toHaveCount(0);
});
