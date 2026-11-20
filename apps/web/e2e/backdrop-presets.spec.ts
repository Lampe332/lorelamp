// Ready-made backdrops (Q4): the gallery in a chat's "Aussehen" with 16 small pictures, picking one
// for this chat or as the character's default, "Kein Hintergrund", the pick after a reload and after
// a restart of the test server, and the gallery on a tablet. Every test works in its own new chats
// and gives the test character's default backdrop back.

import { execSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { api, charId, deleteChat, newChat, setToken } from "./api";
import { dataDir } from "./db";
import { expect, isPhone, test } from "./fixtures";

type Backdrop = { rev: number; dim: number; preset?: string };
type ChatLook = { character_id: number; backdrop: { mode: string; own: Backdrop | null; character: Backdrop | null } };
const chatLook = (id: number) => api<ChatLook>("GET", `/chats/${id}/look`);

const ILSE = "Kapitän Ilse Brandt";
let chats: number[] = [];
// A default left behind by a run that stopped half-way would show in every new chat with her.
test.beforeEach(async () => {
  await api("DELETE", `/characters/${await charId(ILSE)}/backdrop`);
});
test.afterEach(async () => {
  for (const id of chats) {
    await expect(async () => {
      await deleteChat(id);
    }).toPass({ timeout: 60_000, intervals: [1000] });
  }
  chats = [];
  await api("DELETE", `/characters/${await charId(ILSE)}/backdrop`);
});

async function freshChat(): Promise<number> {
  const id = await newChat(await charId(ILSE));
  chats.push(id);
  return id;
}

const shotsDir = process.env.RP_E2E_SHOTS ?? "";
async function shot(page: Page, name: string, project: string) {
  if (!shotsDir) return;
  mkdirSync(shotsDir, { recursive: true });
  await page.screenshot({ path: join(shotsDir, `${name}-${project}.png`) });
}

const panel = (page: Page) => page.getByRole("dialog", { name: "Aussehen dieses Chats" });
async function openLook(page: Page) {
  if (isPhone(page)) await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.getByRole("button", { name: "Aussehen dieses Chats" }).click();
  await expect(panel(page)).toBeVisible();
}
const forChat = (page: Page) => panel(page).getByRole("list", { name: "Hintergrund für diesen Chat" });
const forFigure = (page: Page) => panel(page).getByRole("list", { name: `Standard-Hintergrund für ${ILSE}` });
const tile = (list: ReturnType<typeof forChat>, name: string) => list.getByRole("button", { name, exact: true });
const backdrop = (page: Page) => page.locator(".cv-backdrop img");

/** Shows the gallery in the panel (on a phone the sheet scrolls). */
async function showGallery(page: Page) {
  await panel(page).getByRole("heading", { name: "Vorlagen" }).scrollIntoViewIfNeeded();
  await page.mouse.move(0, 0);
}

test("Vorlagen: 16 Vorschaubilder, Klick setzt den Hintergrund, bleibt nach Neuladen, „Kein Hintergrund“ entfernt ihn", async ({ page, open }, info) => {
  const id = await freshChat();
  await open(`/chat/${id}`);
  await expect(page.locator(".cv-backdrop")).toHaveCount(0);
  await openLook(page);
  const list = forChat(page);
  await expect(list.getByRole("button")).toHaveCount(17); // "Kein Hintergrund" + 16
  await expect(tile(list, "Kein Hintergrund")).toHaveAttribute("aria-pressed", "true");
  // Every thumbnail arrives, small and quick: the 400 px WebP, not the full picture.
  const thumbs = list.locator("img");
  await expect(thumbs).toHaveCount(16);
  for (const img of await thumbs.all()) {
    await img.scrollIntoViewIfNeeded();
    await expect.poll(() => img.evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth)).toBe(400);
  }
  const first = await page.request.get((await thumbs.first().getAttribute("src"))!);
  expect([first.headers()["content-type"], (await first.body()).length < 24 * 1024]).toEqual(["image/webp", true]);
  // Big enough for a finger on a phone.
  const box = (await tile(list, "Bergsee").boundingBox())!;
  expect(box.height).toBeGreaterThanOrEqual(isPhone(page) ? 44 : 40);

  await tile(list, "Bergsee").click();
  await expect(backdrop(page)).toBeVisible();
  await expect.poll(() => backdrop(page).evaluate((el: HTMLImageElement) => el.complete && el.naturalWidth)).toBe(1672);
  expect(await backdrop(page).getAttribute("src")).toContain(`/api/chats/${id}/backdrop?`);
  await expect(tile(list, "Bergsee")).toHaveAttribute("aria-pressed", "true");
  await expect(tile(list, "Kein Hintergrund")).toHaveAttribute("aria-pressed", "false");
  await expect(panel(page).locator(".cv-bg-thumb span")).toContainText("Bergsee · Nur dieser Chat");
  let view = await chatLook(id);
  expect([view.backdrop.mode, view.backdrop.own?.preset]).toEqual(["own", "bergsee"]);
  // Stored as the chat's own JPEG in the data folder, so every device shows the same picture.
  if (dataDir) expect(existsSync(join(dataDir, "backdrops", `chat-${id}.jpg`))).toBe(true);
  await showGallery(page);
  await shot(page, "vorlagen", info.project.name);

  // Dimming a preset keeps it marked.
  await panel(page).getByRole("slider", { name: "Abdunkeln" }).fill("40");
  await expect.poll(async () => (await chatLook(id)).backdrop.own).toMatchObject({ dim: 40, preset: "bergsee" });

  // After a reload the chat still shows it and the gallery still marks it.
  await page.reload();
  await expect(backdrop(page)).toBeVisible();
  await openLook(page);
  await expect(tile(forChat(page), "Bergsee")).toHaveAttribute("aria-pressed", "true");

  // Another one replaces it.
  await tile(forChat(page), "Japanische Gasse").click();
  await expect(tile(forChat(page), "Japanische Gasse")).toHaveAttribute("aria-pressed", "true");
  await expect(tile(forChat(page), "Bergsee")).toHaveAttribute("aria-pressed", "false");
  await expect.poll(async () => (await chatLook(id)).backdrop.own?.preset).toBe("japanische-gasse");

  // "Kein Hintergrund": the chat is plain again.
  await tile(forChat(page), "Kein Hintergrund").click();
  await expect(page.locator(".cv-backdrop")).toHaveCount(0);
  await expect(tile(forChat(page), "Kein Hintergrund")).toHaveAttribute("aria-pressed", "true");
  view = await chatLook(id);
  expect(view.backdrop.mode).toBe("none");

  // An own upload still works next to the gallery and is no preset.
  await expect(panel(page).getByRole("button", { name: "Eigenes Bild ersetzen" })).toBeVisible();
});

test("Figur-Hintergrund: Vorlage für alle Chats mit der Figur, neuer Chat zeigt sie, „Kein Hintergrund“ nimmt sie weg", async ({ page, open }, info) => {
  const id = await freshChat();
  const character = await charId(ILSE);
  await open(`/chat/${id}`);
  await openLook(page);
  await panel(page).getByRole("button", { name: `Alle Chats mit ${ILSE}`, exact: true }).click();
  await expect(forFigure(page)).toBeVisible();
  await expect(tile(forFigure(page), "Kein Hintergrund")).toHaveAttribute("aria-pressed", "true");
  await tile(forFigure(page), "Hafen").click();
  await expect(panel(page).locator(".cv-status")).toContainText(`Chats mit ${ILSE} ohne eigenes Bild zeigen jetzt Hafen.`);
  await expect(backdrop(page)).toBeVisible();
  expect(await backdrop(page).getAttribute("src")).toContain(`/api/characters/${character}/backdrop?`);
  await expect(tile(forFigure(page), "Hafen")).toHaveAttribute("aria-pressed", "true");
  expect((await chatLook(id)).backdrop.character?.preset).toBe("hafen");
  if (dataDir) expect(existsSync(join(dataDir, "backdrops", `char-${character}.jpg`))).toBe(true);
  await showGallery(page);
  await shot(page, "vorlagen-figur", info.project.name);

  // A new chat with her starts with it; "Dieser Chat" marks what this chat shows.
  const second = await freshChat();
  await open(`/chat/${second}`);
  await expect(backdrop(page)).toBeVisible();
  expect(await backdrop(page).getAttribute("src")).toContain(`/api/characters/${character}/backdrop?`);
  await openLook(page);
  await expect(tile(forChat(page), "Hafen")).toHaveAttribute("aria-pressed", "true");
  await expect(panel(page).locator(".cv-bg-thumb span")).toContainText(`Hafen · Standard von ${ILSE}`);

  // This chat picks its own; the character keeps hers.
  await tile(forChat(page), "Schneedorf").click();
  await expect.poll(async () => (await chatLook(second)).backdrop.own?.preset).toBe("schneedorf");
  expect((await chatLook(second)).backdrop.character?.preset).toBe("hafen");

  // "Kein Hintergrund" for the character removes her default; the first chat shows nothing then.
  await panel(page).getByRole("button", { name: `Alle Chats mit ${ILSE}`, exact: true }).click();
  await tile(forFigure(page), "Kein Hintergrund").click();
  await expect(tile(forFigure(page), "Kein Hintergrund")).toHaveAttribute("aria-pressed", "true");
  await expect.poll(async () => (await chatLook(id)).backdrop.character).toBeNull();
  await open(`/chat/${id}`);
  await expect(page.locator(".cv-backdrop")).toHaveCount(0);
});

test("Nach einem Neustart des Servers: Chat- und Figur-Vorlage sind noch gesetzt", async ({ page, open }, info) => {
  test.skip(!process.env.RP_E2E_RESTART, "needs RP_E2E_RESTART (a command that restarts the test server and prints its new address)");
  test.skip(info.project.name !== "desktop-graphit", "one restart per run is enough");
  const id = await freshChat();
  await open(`/chat/${id}`);
  await openLook(page);
  await tile(forChat(page), "Strand").click();
  await expect.poll(async () => (await chatLook(id)).backdrop.own?.preset).toBe("strand");
  await panel(page).getByRole("button", { name: `Alle Chats mit ${ILSE}`, exact: true }).click();
  await tile(forFigure(page), "Magierturm").click();
  await expect.poll(async () => (await chatLook(id)).backdrop.character?.preset).toBe("magierturm");
  // The figure's default shows here now; the chat's own stays stored under "Eigenes".
  await expect.poll(async () => (await chatLook(id)).backdrop.mode).toBe("character");
  await panel(page).getByRole("button", { name: "Eigenes", exact: true }).click();
  await expect.poll(async () => (await chatLook(id)).backdrop.mode).toBe("own");

  // Into a file, not a pipe: the new server inherits the command's output handles and would keep a
  // pipe open for as long as it runs.
  const outFile = join(tmpdir(), `rp-e2e-restart-${process.pid}.txt`);
  execSync(`${process.env.RP_E2E_RESTART} > "${outFile}" 2>&1`, { stdio: "ignore", timeout: 60_000 });
  const out = readFileSync(outFile, "utf8");
  try {
    rmSync(outFile, { force: true });
  } catch {
    // Still held by the new server through its inherited handle; the next run overwrites it.
  }
  const fresh = /token=(\w+)/.exec(out)?.[1];
  expect(fresh, "the restart command prints the server's new address").toBeTruthy();
  setToken(fresh!);
  // toPass, not poll: the first request may still meet a kept-alive socket of the old server.
  await expect(async () => expect((await chatLook(id)).backdrop.own?.preset).toBe("strand")).toPass({ timeout: 30_000 });

  await open(`/chat/${id}`);
  await expect(backdrop(page)).toBeVisible();
  expect(await backdrop(page).getAttribute("src")).toContain(`/api/chats/${id}/backdrop?`);
  await openLook(page);
  await expect(tile(forChat(page), "Strand")).toHaveAttribute("aria-pressed", "true");
  await panel(page).getByRole("button", { name: `Alle Chats mit ${ILSE}`, exact: true }).click();
  await expect(tile(forFigure(page), "Magierturm")).toHaveAttribute("aria-pressed", "true");
  expect((await chatLook(id)).backdrop.character?.preset).toBe("magierturm");
});

test.describe("Tablet", () => {
  test.use({ viewport: { width: 800, height: 1280 }, hasTouch: true, isMobile: true });

  test("Vorlagen am Tablet: Ziele mindestens 48 px, Tippen setzt den Hintergrund", async ({ page, open }, info) => {
    test.skip(info.project.name.startsWith("phone-"), "the tablet window is set here; one run per theme");
    const id = await freshChat();
    await open(`/chat/${id}`);
    await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
    await openLook(page);
    const list = forChat(page);
    for (const name of ["Kein Hintergrund", "Taverne", "Magierturm"]) {
      const b = (await tile(list, name).boundingBox())!;
      expect(b.height).toBeGreaterThanOrEqual(48);
    }
    for (const name of ["Dieser Chat", `Alle Chats mit ${ILSE}`]) {
      const b = (await panel(page).getByRole("button", { name, exact: true }).boundingBox())!;
      expect(b.height).toBeGreaterThanOrEqual(48);
    }
    await tile(list, "Taverne").tap();
    await expect(backdrop(page)).toBeVisible();
    await expect(tile(list, "Taverne")).toHaveAttribute("aria-pressed", "true");
    await showGallery(page);
    await shot(page, "vorlagen-tablet", info.project.name);
  });
});
