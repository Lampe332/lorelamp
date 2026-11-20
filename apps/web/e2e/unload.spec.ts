// K7 freeing the graphics memory: "Entladen" next to the model status stops the model server (its
// process is gone), the status says so, and the next message loads the model again and gets its
// answer; Ctrl+K unloads and loads it; the quiet time of the tab "Modell" unloads it by itself.
// Needs a test server that manages the model (`--manage-engine`, see README); skips otherwise. The
// quiet-time test needs the server started with RP_IDLE_MINUTE_SECS (a shortened minute) and the
// same value in RP_E2E_IDLE_MINUTE_SECS.

import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { api, charId, deleteChat, newChat } from "./api";
import { expect, isPhone, modelStatus, test } from "./fixtures";

type Engine = {
  status: string;
  managed: boolean;
  unloaded: boolean;
  idle_unload_min: number;
  pid: number | null;
};
const engine = () => api<Engine>("GET", "/engine");
const SHOTS = process.env.RP_E2E_SHOTS;
const MINUTE_SECS = Number(process.env.RP_E2E_IDLE_MINUTE_SECS ?? 0);

/** Is a process with this id still running? (Windows: tasklist.) */
function alive(pid: number): boolean {
  return execFileSync("tasklist", ["/FI", `PID eq ${pid}`, "/NH"], {
    encoding: "utf8",
  }).includes(String(pid));
}

async function ready(): Promise<Engine> {
  let last: Engine | null = null;
  await expect
    .poll(async () => (last = await engine()).status === "ready" && !last.unloaded && last.pid !== null, { timeout: 120_000, intervals: [500, 1000, 2000] })
    .toBe(true);
  return last!;
}

async function unloadedNow(): Promise<void> {
  await expect
    .poll(
      async () => {
        const e = await engine();
        return e.unloaded && e.pid === null;
      },
      { timeout: 30_000 },
    )
    .toBe(true);
}

async function shot(page: Page, name: string, theme: string) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({
    path: join(SHOTS, `${name}-${isPhone(page) ? "375" : "1920"}-${theme}.png`),
  });
}

let chats: number[] = [];

test.beforeEach(async () => {
  test.skip(!(await engine()).managed, "the test server does not manage the model (start it with --manage-engine)");
});

// The model is loaded again and the quiet time is off after every test.
test.afterEach(async () => {
  await api("PUT", "/engine/idle", { minutes: 0 });
  if ((await engine()).unloaded) await api("POST", "/engine/load");
  await ready();
  for (const id of chats)
    await expect(async () => void (await deleteChat(id))).toPass({
      timeout: 120_000,
      intervals: [1000],
    });
  chats = [];
});

const palette = (page: Page) => page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });

async function quickAction(page: Page, words: string, label: string) {
  if (isPhone(page)) await page.locator("header.top").getByRole("button", { name: "Suchen und Schnellaktionen" }).click();
  else await page.keyboard.press("Control+k");
  await expect(palette(page)).toBeVisible();
  await page.keyboard.type(words);
  await palette(page).getByRole("option").filter({ hasText: label }).first().click();
}

test("Entladen: Knopf am Modell-Status, Prozess weg, Senden lädt wieder und antwortet", async ({ page, open, theme }) => {
  test.skip(isPhone(page), "the button sits in the sidebar (desktop); phones use Ctrl+K, see below");
  test.setTimeout(240_000);
  const before = await ready();
  await open("/");
  const unload = page.getByRole("button", { name: "Modell entladen" });
  await expect(unload).toHaveAttribute("title", /Grafikspeicher frei/);
  await unload.click();
  await unloadedNow();
  expect(alive(before.pid!), "no llama-server process left").toBe(false);
  const pill = modelStatus(page);
  await expect(pill).toContainText("Entladen");
  await expect(pill).toContainText("lädt beim nächsten Senden");
  await expect(page.getByRole("button", { name: "Modell laden" })).toBeVisible();
  await shot(page, "seitenleiste-entladen", theme);

  // Sending loads the model: the status says so, then the answer comes as usual.
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  await open(`/chat/${id}`);
  await page.getByRole("textbox", { name: "Nachricht an Mara Voss" }).fill("Hallo, wie geht es dir?");
  await page.getByRole("button", { name: "Senden" }).click();
  await expect(pill).toContainText(/Modell lädt|Modell startet|Modell bereit/, {
    timeout: 30_000,
  });
  const answer = page.getByRole("article", { name: "Nachricht der Figur" }).last();
  await expect(answer.getByRole("button", { name: "Text kopieren" })).toBeVisible({ timeout: 180_000 });
  await expect(page.getByRole("alert")).toHaveCount(0);
  const after = await ready();
  expect(after.pid).not.toBe(before.pid);
  await expect(page.getByRole("button", { name: "Modell entladen" })).toBeVisible();
});

test("Strg K: Grafikspeicher freigeben, Modell laden", async ({ page, open }) => {
  test.setTimeout(180_000);
  await ready();
  await open("/");
  await quickAction(page, "grafikspeicher", "Grafikspeicher freigeben");
  await unloadedNow();
  await expect(modelStatus(page)).toContainText("Entladen");
  await quickAction(page, "modell laden", "Modell laden");
  await ready();
  await expect(modelStatus(page)).toContainText("Modell bereit");
});

test("Tab Modell: Modell nach … entladen (Standard aus) entlädt nach der Leerlaufzeit", async ({ page, open, theme }) => {
  test.setTimeout(180_000);
  expect((await engine()).idle_unload_min, "off by default").toBe(0);
  await ready();
  await open("/settings/model");
  const choice = page.getByLabel("Modell entladen nach");
  await expect(choice).toHaveValue("0");
  await expect(page.locator('[data-spot="idle-unload"]').getByRole("status")).toContainText("Geladen");
  await page.locator('[data-spot="idle-unload"]').scrollIntoViewIfNeeded();
  await shot(page, "tab-modell-grafikspeicher", theme);
  test.skip(!MINUTE_SECS, "the quiet time needs a shortened minute (RP_IDLE_MINUTE_SECS on the server, RP_E2E_IDLE_MINUTE_SECS here)");
  await choice.selectOption("5");
  await expect.poll(async () => (await engine()).idle_unload_min).toBe(5);
  // 5 minutes of MINUTE_SECS seconds, plus the supervisor's 3 s check.
  await expect
    .poll(async () => (await engine()).unloaded, {
      timeout: (5 * MINUTE_SECS + 20) * 1000,
      intervals: [1000],
    })
    .toBe(true);
  await expect(page.locator('[data-spot="idle-unload"]').getByRole("status")).toContainText("Entladen – lädt beim nächsten Senden");
  await shot(page, "tab-modell-entladen", theme);
  await page.locator('[data-spot="idle-unload"]').getByRole("button", { name: "Modell laden" }).click();
  await ready();
});
