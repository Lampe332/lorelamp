// K6 context meter in the chat header: tokens of the newest measured answer (read new + from the
// cache + written) against the model's context from /engine; warning colour from 85 %.

import { api, charId, deleteChat, newChat } from "./api";
import { fillChat, messageIds, setTimings } from "./db";
import { expect, isPhone, test } from "./fixtures";

let chat = 0;
test.afterEach(async () => {
  if (chat) await deleteChat(chat);
  chat = 0;
});

test("Kontext-Anzeige: belegt / gesamt, Warnfarbe ab 85 %", async ({ page, open }) => {
  const { model } = await api<{ model: { ctx: number | null } }>("GET", "/engine");
  test.skip(!model.ctx, "the model reports no context");
  const ctx = model.ctx!;
  chat = await newChat(await charId("Mara Voss"));
  fillChat(chat, 5);
  const last = messageIds(chat)[4]; // message 5: the character's
  const used = Math.round(ctx * 0.5);
  setTimings(last, { tok_s: 20, processed: 40, cached: used - 140, predicted: 100 });

  await open(`/chat/${chat}`);
  const meter = page.getByRole("meter", { name: "Kontext belegt" });
  await expect(meter).toHaveAttribute("aria-valuenow", String(used));
  await expect(meter).toHaveAttribute("aria-valuemax", String(ctx));
  await expect(meter).not.toHaveClass(/warn/);
  if (isPhone(page)) await expect(page.locator(".ctx-inline")).toHaveText("Kontext 50 % · ");
  else await expect(meter).toContainText(" / ");

  // 90 %: the warning colour.
  setTimings(last, { tok_s: 20, processed: 40, cached: Math.round(ctx * 0.9) - 140, predicted: 100 });
  await page.reload();
  await expect(meter).toHaveClass(/warn/);
  const colors = await page.evaluate(() => {
    const bar = document.querySelector(".ctx-meter i b") as HTMLElement;
    const probe = document.createElement("i");
    probe.style.color = "var(--warn)";
    document.body.append(probe);
    const warn = getComputedStyle(probe).color;
    probe.remove();
    return { bar: getComputedStyle(bar).backgroundColor, warn };
  });
  expect(colors.bar).toBe(colors.warn);
});
