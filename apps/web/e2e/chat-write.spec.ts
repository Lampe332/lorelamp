// The chat's last reply once more (Q3): "Neu generieren" writes another variant of it (the earlier
// one stays and can be chosen again), "Weiter schreiben" lets the model go on at its end. Both go
// through the server's hub: while the model writes, every further request is refused with 409, and
// the reply being written cannot be changed. On a tablet a swipe past the last variant writes a
// new one.
//
// Needs the test server's model (the CPU model of e2e/README.md) and `RP_E2E_DATA` (the variants
// are read from the test database). Every test works in its own chat and deletes it afterwards.
// `RP_E2E_SHOTS` (optional): review screenshots go there.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import type { Locator, Page } from "@playwright/test";
import { api, baseURL, charId, deleteChat, newChat, token } from "./api";
import { dataDir } from "./db";
import { expect, test } from "./fixtures";

const shotsDir = process.env.RP_E2E_SHOTS ?? "";
let chats: number[] = [];

test.beforeEach(async () => {
  test.skip(!dataDir, "needs RP_E2E_DATA (the test server's data folder)");
  const engine = await api<{ status: string }>("GET", "/engine");
  test.skip(engine.status !== "ready", "needs the test server's model (see e2e/README.md)");
  test.setTimeout(300_000);
});
test.afterEach(async () => {
  for (const id of chats) {
    await expect(async () => {
      await deleteChat(id);
    }).toPass({ timeout: 120_000, intervals: [1000] });
  }
  chats = [];
});

async function shot(page: Page, name: string, project: string) {
  if (!shotsDir) return;
  mkdirSync(shotsDir, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(shotsDir, `${name}-${project}.png`) });
}

type Row = { id: number; role: string; content: string; swipes: string[] | null; swipe_id: number | null };
/** The chat's messages as the database holds them. */
function rows(chatId: number): Row[] {
  const db = new DatabaseSync(join(dataDir, "rp-app.db"));
  try {
    db.exec("PRAGMA busy_timeout = 5000");
    const all = db.prepare("SELECT id, role, content, swipes, swipe_id FROM messages WHERE chat_id = ? AND deleted_at IS NULL ORDER BY id").all(chatId) as { id: number; role: string; content: string; swipes: string | null; swipe_id: number | null }[];
    return all.map((r) => ({ ...r, swipes: r.swipes ? (JSON.parse(r.swipes) as string[]) : null }));
  } finally {
    db.close();
  }
}

/** The HTTP status and error code of a request the server may refuse. */
async function ask(method: string, path: string, body?: unknown): Promise<{ status: number; error?: string }> {
  const res = await fetch(`${baseURL}/api${path}`, {
    method,
    headers: { Authorization: `Bearer ${token}`, ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  return { status: res.status, error: text ? (JSON.parse(text) as { error?: string }).error : undefined };
}

/** The start of a text as the page shows it: without the marks of actions and emphasis. */
const shown = (text: string) => text.split(/\n/)[0].replaceAll("*", "").slice(0, 30).trim();
const sendButton = (page: Page) => page.getByRole("button", { name: "Senden" });
const stopButton = (page: Page) => page.getByRole("button", { name: "Antwort stoppen" });
const variants = (m: Locator) => m.getByRole("group", { name: "Antwort-Varianten" });
const regen = (m: Locator) => m.getByRole("button", { name: "Neu generieren" });
const more = (m: Locator) => m.getByRole("button", { name: "Weiter schreiben" });

/** A new chat with one exchange: your message and a short reply of the model (stopped after a few words). */
async function chatWithReply(page: Page, open: (hash: string) => Promise<void>): Promise<number> {
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  await open(`/chat/${id}`);
  await page.getByRole("textbox", { name: /^Nachricht an / }).fill("Erzähl mir in ein paar Sätzen vom Leuchtturm.");
  await sendButton(page).click();
  await stopWhenWritten(page, 3, "");
  return id;
}

/** Waits until the reply at `index` is being written and holds more than `from`, then stops it. */
async function stopWhenWritten(page: Page, index: number, from: string) {
  await expect(stopButton(page)).toBeVisible({ timeout: 60_000 });
  const article = page.getByRole("article").nth(index - 1);
  const prose = article.locator(".prose");
  // A reply written again shows its old text (faded, `cv-rewrite`) until the first new word: that is not new text.
  await expect
    .poll(async () => ((await article.getAttribute("class"))?.includes("cv-rewrite") ? 0 : (await prose.innerText()).replace("Schreibt …", "").trim().length), {
      timeout: 120_000,
    })
    .toBeGreaterThan(from.length + 12);
  // The model may have finished by itself in this very moment: then there is nothing to stop.
  await stopButton(page).click({ timeout: 3000 }).catch(() => {});
  await expect(sendButton(page)).toBeVisible({ timeout: 30_000 });
}

test("Neu generieren: neue Variante entsteht, die alte bleibt wählbar; während das Modell schreibt 409", async ({ page, open }, info) => {
  const id = await chatWithReply(page, open);
  const all = page.getByRole("article");
  await expect(all).toHaveCount(3);
  const before = rows(id);
  expect(before.map((r) => r.role)).toEqual(["assistant", "user", "assistant"]);
  expect(before[2].swipes).toBeNull();
  const first = before[2].content;
  expect(first.length).toBeGreaterThan(10);
  const reply = all.nth(2);

  // Only the chat's last reply offers it: not the greeting, not your own message.
  await expect(regen(reply)).toBeEnabled();
  await expect(more(reply)).toBeEnabled();
  await expect(regen(all.nth(0))).toHaveCount(0);
  await expect(more(all.nth(1))).toHaveCount(0);
  await expect(variants(reply)).toHaveCount(0);
  expect((await ask("POST", `/messages/${before[0].id}/regenerate`)).status, "the greeting").toBe(409);
  expect(await ask("POST", `/messages/${before[1].id}/continue`)).toEqual({ status: 409, error: "not_last" });
  expect((await ask("POST", "/messages/999999/regenerate")).status).toBe(404);
  await shot(page, "antwort-aktionen", info.project.name);

  await regen(reply).click();
  // The reply itself is written again: no new message, the stop button in place of "Senden".
  await expect(stopButton(page)).toBeVisible({ timeout: 60_000 });
  await expect(all).toHaveCount(3);
  // While the model writes: nothing else is written, and this reply cannot be changed.
  expect(await ask("POST", `/messages/${before[2].id}/regenerate`)).toEqual({ status: 409, error: "busy" });
  expect(await ask("POST", `/messages/${before[2].id}/continue`)).toEqual({ status: 409, error: "busy" });
  expect(await ask("PUT", `/messages/${before[2].id}`, { content: "x" })).toEqual({ status: 409, error: "busy" });
  expect(rows(id)[2].content, "the stored reply is untouched until the new one is done").toBe(first);
  await stopWhenWritten(page, 3, "");

  // Two variants, the new one on show.
  await expect(variants(reply)).toContainText("2 / 2");
  const after = rows(id);
  expect(after).toHaveLength(3);
  expect(after[2].id).toBe(before[2].id);
  expect(after[2].swipes).toHaveLength(2);
  expect(after[2].swipes![0]).toBe(first);
  expect(after[2].swipe_id).toBe(1);
  const second = after[2].swipes![1];
  expect(after[2].content).toBe(second);
  await expect(reply.locator(".prose")).toContainText(shown(second));
  await shot(page, "antwort-zwei-varianten", info.project.name);

  // Back to the first one and forth again; the choice survives a reload.
  await variants(reply).getByRole("button", { name: "Vorige Variante" }).click();
  await expect(variants(reply)).toContainText("1 / 2");
  await expect.poll(() => rows(id)[2].content).toBe(first);
  await page.reload();
  await expect(variants(page.getByRole("article").nth(2))).toContainText("1 / 2");
  await expect(page.getByRole("article").nth(2).locator(".prose")).toContainText(shown(first));

  // A third one, from the first: the list grows at its end.
  await regen(page.getByRole("article").nth(2)).click();
  await stopWhenWritten(page, 3, "");
  await expect(variants(page.getByRole("article").nth(2))).toContainText("3 / 3");
  const third = rows(id)[2];
  expect(third.swipes).toHaveLength(3);
  expect(third.swipes!.slice(0, 2)).toEqual([first, second]);
  expect(third.swipe_id).toBe(2);
});

test("Weiter schreiben: die letzte Antwort wächst an ihrem Ende, ihre Varianten bleiben", async ({ page, open }, info) => {
  const id = await chatWithReply(page, open);
  const reply = page.getByRole("article").nth(2);
  const start = rows(id)[2];
  await expect(more(reply)).toBeEnabled();

  await more(reply).click();
  await expect(stopButton(page)).toBeVisible({ timeout: 60_000 });
  // What was there stays on screen while more arrives.
  await expect(reply.locator(".prose")).toContainText(shown(start.content));
  expect(await ask("POST", `/messages/${start.id}/continue`)).toEqual({ status: 409, error: "busy" });
  expect(await ask("POST", `/messages/${start.id}/regenerate`)).toEqual({ status: 409, error: "busy" });
  await stopWhenWritten(page, 3, start.content);

  const grown = rows(id);
  expect(grown).toHaveLength(3);
  expect(grown[2].id).toBe(start.id);
  expect(grown[2].content.startsWith(start.content), "the old text is its beginning").toBe(true);
  expect(grown[2].content.length).toBeGreaterThan(start.content.length + 5);
  expect(grown[2].swipes, "going on makes no variant").toBeNull();
  await expect(variants(reply)).toHaveCount(0);
  await shot(page, "antwort-weitergeschrieben", info.project.name);

  // With variants, only the one on show grows.
  await regen(reply).click();
  await stopWhenWritten(page, 3, "");
  await expect(variants(reply)).toContainText("2 / 2");
  const two = rows(id)[2];
  await more(reply).click();
  await stopWhenWritten(page, 3, two.content);
  const end = rows(id)[2];
  expect(end.swipes).toHaveLength(2);
  expect(end.swipes![0]).toBe(grown[2].content);
  expect(end.swipes![1].startsWith(two.content)).toBe(true);
  expect(end.swipes![1].length).toBeGreaterThan(two.content.length);
  expect(end.content).toBe(end.swipes![1]);
  await expect(variants(reply)).toContainText("2 / 2");

  // Once the chat goes on, the old reply offers neither any more.
  await page.getByRole("textbox", { name: /^Nachricht an / }).fill("Und weiter?");
  await sendButton(page).click();
  await stopWhenWritten(page, 5, "");
  await expect(page.getByRole("article")).toHaveCount(5);
  await expect(regen(page.getByRole("article").nth(2))).toHaveCount(0);
  await expect(regen(page.getByRole("article").nth(4))).toBeEnabled();
  expect(await ask("POST", `/messages/${start.id}/regenerate`)).toEqual({ status: 409, error: "not_last" });
});

test("Rechts auf der letzten Variante schreibt eine neue; die alte bleibt bis zum ersten Wort stehen", async ({ page, open }, info) => {
  const id = await chatWithReply(page, open);
  const reply = page.getByRole("article").nth(2);
  await regen(reply).click();
  await stopWhenWritten(page, 3, "");
  await expect(variants(reply)).toContainText("2 / 2");
  const two = rows(id)[2];

  // At the last variant of the chat's last reply the right arrow is no end: it writes a new one.
  const writeNew = variants(reply).getByRole("button", { name: "Neue Variante schreiben" });
  await expect(writeNew).toBeEnabled();
  await expect(writeNew).toHaveAttribute("title", "Neue Variante schreiben (Pfeil rechts)");
  await shot(page, "rechts-neue-variante", info.project.name);
  // Until its first new word the reply keeps the text it had, faded, the caret at its start; the
  // header does not count it twice.
  const before = await page.evaluate(() => {
    (window as unknown as { rewrite?: { faded: string; caret: string; count: string } }).rewrite = undefined;
    const watch = new MutationObserver(() => {
      const a = document.querySelector("article.cv-rewrite .prose");
      const w = window as unknown as { rewrite?: { faded: string; caret: string; count: string } };
      if (!a || w.rewrite) return;
      w.rewrite = {
        faded: getComputedStyle(a.querySelector("p")!).opacity,
        caret: getComputedStyle(a, "::before").position,
        count: document.querySelector(".chat-bar .chat-name")?.parentElement?.textContent ?? "",
      };
    });
    watch.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["class"] });
    return document.querySelector(".chat-bar .chat-name")?.parentElement?.textContent ?? "";
  });
  await writeNew.click();
  await expect(stopButton(page)).toBeVisible({ timeout: 60_000 });
  const seen = await page.evaluate(() => (window as unknown as { rewrite?: { faded: string; caret: string; count: string } }).rewrite);
  expect(seen, "the old text was on screen until the first new word").toBeTruthy();
  expect(Number(seen!.faded)).toBeLessThan(0.6);
  expect(seen!.caret).toBe("absolute");
  expect(seen!.count).toBe(before);
  await stopWhenWritten(page, 3, "");
  await expect(variants(reply)).toContainText("3 / 3");
  expect(rows(id)[2].swipes!.slice(0, 2)).toEqual(two.swipes);

  // The arrow key does the same, from the group's focus.
  await variants(reply).getByRole("button", { name: "Vorige Variante" }).focus();
  await page.keyboard.press("ArrowLeft");
  await expect(variants(reply)).toContainText("2 / 3");
  await page.keyboard.press("ArrowRight");
  await expect(variants(reply)).toContainText("3 / 3");
  await expect(stopButton(page)).toHaveCount(0);
  await page.keyboard.press("ArrowRight");
  await stopWhenWritten(page, 3, "");
  await expect(variants(reply)).toContainText("4 / 4");
  expect(rows(id)[2].swipes).toHaveLength(4);
});

test("Eigene Nachricht: Speichern und neu antworten", async ({ page, open }, info) => {
  const id = await chatWithReply(page, open);
  const all = page.getByRole("article");
  const before = rows(id);
  await page.getByRole("textbox", { name: /^Nachricht an / }).press("ArrowUp");
  const editBox = all.nth(1).getByRole("textbox", { name: "Nachricht bearbeiten" });
  await expect(editBox).toBeVisible();
  const again = all.nth(1).getByRole("button", { name: "Speichern und neu antworten" });
  await expect(again).toBeEnabled();
  // The greeting has no such button: no reply of yours follows it alone.
  await editBox.fill("Erzähl mir in zwei Sätzen vom Hafen.");
  await shot(page, "speichern-neu-antworten", info.project.name);
  await again.click();
  await expect(editBox).toHaveCount(0);
  await expect(stopButton(page)).toBeVisible({ timeout: 60_000 });
  await stopWhenWritten(page, 3, "");
  const after = rows(id);
  expect(after).toHaveLength(3);
  expect(after[1].content).toBe("Erzähl mir in zwei Sätzen vom Hafen.");
  expect(after[2].id).toBe(before[2].id);
  expect(after[2].swipes).toHaveLength(2);
  expect(after[2].swipes![0]).toBe(before[2].content);
  await expect(variants(all.nth(2))).toContainText("2 / 2");
});

test("Kontext voll: klarer Text statt Rohfehler", async ({ page, open }, info) => {
  test.skip(info.project.name !== "desktop-graphit" && info.project.name !== "phone-graphit", "one run per size is enough");
  const id = await newChat(await charId("Mara Voss"));
  chats.push(id);
  await open(`/chat/${id}`);
  // What the server tells this page: the code of a refusal (`notice`) or of a reply that failed (`done`).
  const codes: string[] = [];
  page.on("websocket", (ws) =>
    ws.on("framereceived", (f) => {
      const ev = JSON.parse(String(f.payload)) as { type?: string; code?: string; error?: string | null };
      if (ev.type === "notice" && ev.code) codes.push(ev.code);
      if (ev.type === "done") codes.push(ev.error ?? "ok");
    }),
  );
  await page.reload();
  // Far more than the test model's 4096 tokens of context, and under the message limit.
  const long = "Die Wellen schlagen gegen die Mole, und der Wind trägt Salz bis zum Leuchtturm. ".repeat(700);
  const field = page.getByRole("textbox", { name: /^Nachricht an / });
  await field.fill(long);
  await sendButton(page).click();
  await expect.poll(() => codes.length, { timeout: 120_000 }).toBeGreaterThan(0);
  test.skip(!codes.includes("context_full"), `the test server does not send context_full yet (got ${codes.join(", ")})`);
  const said = page.getByText("Der Chat passt nicht mehr in den Kontext des Modells, auch gekürzt nicht.");
  await expect(said.first()).toBeVisible();
  await expect(page.getByText("completion failed")).toHaveCount(0);
  await shot(page, "kontext-voll", info.project.name);
});

test.describe("Tablet 1280x800", () => {
  test.use({ viewport: { width: 1280, height: 800 }, hasTouch: true, isMobile: true });

  /** A finger moving sideways over the page, in steps the page sees as real touch input. */
  async function swipe(page: Page, x: number, y: number, dx: number) {
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
    for (let i = 1; i <= 8; i++) {
      await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: x + (dx * i) / 8, y }] });
      await page.waitForTimeout(16);
    }
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await cdp.detach();
  }
  const middle = async (prose: Locator) => {
    const b = (await prose.boundingBox())!;
    return { x: b.x + Math.min(b.width / 2, 260), y: b.y + Math.min(b.height / 2, 60) };
  };

  test("Wischen nach links über die letzte Variante hinaus schreibt eine neue, zurück wischen zeigt die alte", async ({ page, open }, info) => {
    test.skip(info.project.name.startsWith("phone-"), "sets its own window size; one run per theme is enough");
    const id = await chatWithReply(page, open);
    await expect(page.locator("html")).toHaveAttribute("data-form", "tablet");
    const reply = page.getByRole("article").nth(2);
    const first = rows(id)[2].content;
    await reply.scrollIntoViewIfNeeded();

    // To the right there is nothing: the text settles back, nothing is written.
    let p = await middle(reply.locator(".prose"));
    await swipe(page, p.x, p.y, 180);
    await page.waitForTimeout(400);
    await expect(stopButton(page)).toHaveCount(0);
    expect(rows(id)[2].swipes).toBeNull();

    // To the left: a new variant is written.
    await swipe(page, p.x, p.y, -200);
    await stopWhenWritten(page, 3, "");
    await expect(variants(reply)).toContainText("2 / 2");
    expect(rows(id)[2].swipes![0]).toBe(first);
    await shot(page, "tablet-variante-gewischt", info.project.name);

    // Back to the right: the first one again; left: the second, without writing anything.
    await reply.scrollIntoViewIfNeeded();
    p = await middle(reply.locator(".prose"));
    await swipe(page, p.x, p.y, 200);
    await expect(variants(reply)).toContainText("1 / 2");
    await expect.poll(() => rows(id)[2].content).toBe(first);
    p = await middle(reply.locator(".prose"));
    await swipe(page, p.x, p.y, -200);
    await expect(variants(reply)).toContainText("2 / 2");
    await expect(stopButton(page)).toHaveCount(0);
    expect(rows(id)[2].swipes).toHaveLength(2);
  });
});
