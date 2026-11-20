// Settings › Sicherungen, room and clearing out: the limit (saved, kept after a reload; a smaller one
// asks first and removes the oldest automatic backups), the quiet hint when the folder is nearly full,
// deleting several at once (all or none, cancel keeps them) and "automatic ones older than N days"
// (manual ones stay). Old and large backups are files the test puts into `<data>/backups/` under the
// server's own names; everything a test adds is removed again and the limit goes back to 1 GB.

import { closeSync, copyFileSync, existsSync, ftruncateSync, mkdirSync, openSync, rmSync } from "node:fs";
import { join } from "node:path";
import { api } from "./api";
import { dataDir } from "./db";
import type { Page } from "@playwright/test";
import { expect, isPhone, test } from "./fixtures";

type Backup = { name: string; reason: string; kind: string; created_at: number };
type Overview = { backups: Backup[]; total_bytes: number; limit_bytes: number; near_full: boolean };

const MB = 1024 * 1024;
const GB = 1024 * MB;
const overview = () => api<Overview>("GET", "/backups");
const names = async () => new Set((await overview()).backups.map((b) => b.name));
const backupNow = () => api<Backup>("POST", "/backups");
const shots = process.env.RP_E2E_SHOTS ?? "";
const pickBar = (page: Page) => page.locator('[data-spot="backup-pick"]');

/** A file in the backups folder under an automatic or manual name from 2020, as the server writes them. */
const backupsDir = () => join(dataDir, "backups");
function placeOld(name: string, from?: string, size?: number) {
  mkdirSync(backupsDir(), { recursive: true });
  const path = join(backupsDir(), name);
  if (from) copyFileSync(join(backupsDir(), from), path);
  else {
    const fd = openSync(path, "w");
    ftruncateSync(fd, size ?? 0);
    closeSync(fd);
  }
  return path;
}

let before = new Set<string>();
const placed: string[] = [];
test.beforeEach(async () => {
  before = await names();
});
test.afterEach(async () => {
  for (const p of placed.splice(0)) rmSync(p, { force: true });
  for (const name of await names()) if (!before.has(name)) await api("DELETE", `/backups/${encodeURIComponent(name)}`);
  await api("PUT", "/backups/limit", { limit_bytes: GB });
});

test("Grenze: wählen, gespeichert, bleibt nach Neuladen", async ({ page, open }) => {
  await open("/settings/backups");
  const group = page.getByRole("radiogroup", { name: "Grenze" });
  await expect(group.getByRole("radio")).toHaveCount(4);
  await expect(group.getByRole("radio", { name: "1 GB" })).toBeChecked();

  const [put] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/backups/limit") && r.request().method() === "PUT"),
    group.locator("label", { hasText: "2 GB" }).click(),
  ]);
  expect(put.request().postDataJSON()).toEqual({ limit_bytes: 2 * GB });
  await expect(page.getByRole("status").filter({ hasText: "Grenze gespeichert: 2 GB." })).toBeVisible();
  await expect(page.getByRole("meter", { name: "Platz für Sicherungen" })).toHaveAttribute("aria-valuemax", String(2 * GB));
  await expect(page.locator(".bk-room-used")).toHaveText(/ von 2 GB$/);
  expect((await overview()).limit_bytes).toBe(2 * GB);

  await page.reload();
  await expect(page.getByRole("radiogroup", { name: "Grenze" }).getByRole("radio", { name: "2 GB" })).toBeChecked();
  await expect(page.locator(".bk-room-used")).toHaveText(/ von 2 GB$/);
});

test("Grenze senken fragt mit Anzahl und löscht die ältesten automatischen", async ({ page, open }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA for a large old backup");
  // 520 MB of an old automatic backup: at 500 MB it has to go, at 1 GB it stays.
  const big = "rp-app-2020-01-02-120000-migration.db";
  placed.push(placeOld(big, undefined, 520 * MB));
  await open("/settings/backups");
  const group = page.getByRole("radiogroup", { name: "Grenze" });
  const dialog = page.getByRole("dialog", { name: "Grenze auf 500 MB senken?" });

  await group.locator("label", { hasText: "500 MB" }).click();
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("die älteste automatische Sicherung");
  await expect(dialog).toContainText("Das kann nicht rückgängig gemacht werden.");
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  await expect(dialog).toBeHidden();
  await expect(group.getByRole("radio", { name: "1 GB" })).toBeChecked();
  expect((await overview()).limit_bytes).toBe(GB);
  expect(existsSync(join(backupsDir(), big))).toBe(true);

  await group.locator("label", { hasText: "500 MB" }).click();
  const [put] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/backups/limit") && r.request().method() === "PUT"),
    dialog.getByRole("button", { name: "Grenze senken" }).click(),
  ]);
  expect(await put.json()).toEqual({ limit_bytes: 500 * MB, pruned: 1 });
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("status").filter({ hasText: "Grenze gespeichert: 500 MB. Dafür wurde 1 ältere automatische Sicherung gelöscht." })).toBeVisible();
  await expect(group.getByRole("radio", { name: "500 MB" })).toBeChecked();
  expect(existsSync(join(backupsDir(), big))).toBe(false);
});

test("Fast voll: ruhiger Hinweis mit belegt und Grenze", async ({ page, open, theme }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA for a large old backup");
  await backupNow();
  // 940 MB more against 1 GB: above 90 %.
  placed.push(placeOld("rp-app-2020-01-05-120000-migration.db", undefined, 940 * MB));
  const ov = await overview();
  expect(ov.near_full).toBe(true);
  await open("/settings/backups");
  const hint = page.locator(".x4-near");
  await expect(hint).toBeVisible();
  await expect(hint).toContainText(/^Fast voll: \d+ % der Grenze sind belegt\./);
  await expect(page.locator(".bk-room-used")).toHaveText(/^\d+(,\d)? (MB|GB) von 1 GB$/);
  await expect(page.locator(".bk-meter")).toHaveClass(/high/);
  // Quiet: neither an alert nor a live region.
  await expect(page.getByRole("alert")).toHaveCount(0);
  if (shots) {
    const w = isPhone(page) ? 375 : 1920;
    await page.evaluate(() => document.fonts.ready);
    const top = (sel: string) => page.locator(sel).evaluate((el) => el.scrollIntoView({ block: "start" }));
    await page.screenshot({ path: join(shots, `backups-${w}-${theme}.png`) });
    await top('[data-spot="backup-room"]');
    await page.screenshot({ path: join(shots, `backups-room-${w}-${theme}.png`) });
    await page.getByRole("button", { name: "Mehrere löschen" }).click();
    await page.getByRole("list", { name: "Manuell" }).getByRole("checkbox").first().click();
    const last = page.getByRole("list", { name: "Vor Updates" }).getByRole("checkbox").first();
    await last.click();
    await expect(last).toBeChecked();
    await top('[data-spot="backup-pick"]');
    await page.mouse.move(0, 0);
    await page.waitForTimeout(300); // the boxes' fill fades in
    await page.screenshot({ path: join(shots, `backups-picking-${w}-${theme}.png`) });
    await pickBar(page).getByRole("button", { name: /^Löschen …$/ }).click();
    await page.waitForTimeout(300); // the dialog's entrance
    await page.screenshot({ path: join(shots, `backups-dialog-${w}-${theme}.png`) });
    await page.getByRole("dialog").getByRole("button", { name: "Abbrechen" }).click();
  }
});

/** Three manual backups; the list shows them newest first. */
async function threeManual() {
  const made = [await backupNow(), await backupNow(), await backupNow()];
  return made.map((b) => b.name);
}

test("Mehrere löschen: genau die zwei gewählten gehen, alle/keine", async ({ page, open }) => {
  const [a, b, c] = await threeManual();
  await open("/settings/backups");
  await page.getByRole("button", { name: "Mehrere löschen" }).click();
  const manual = page.getByRole("list", { name: "Manuell" });
  const boxes = manual.getByRole("checkbox");
  const deletable = (await overview()).backups.filter((x) => x.reason !== "hand").length;
  const count = page.locator(".x4-picked");
  await expect(count).toHaveText(`0 von ${deletable} ausgewählt`);

  await page.getByRole("button", { name: "Alle", exact: true }).click();
  await expect(count).toHaveText(`${deletable} von ${deletable} ausgewählt`);
  await page.getByRole("button", { name: "Keine", exact: true }).click();
  await expect(count).toHaveText(`0 von ${deletable} ausgewählt`);

  // Newest first: c, b, a. Pick c and a.
  await boxes.nth(0).click();
  await boxes.nth(2).click();
  await expect(boxes.nth(0)).toBeChecked();
  await expect(boxes.nth(1)).not.toBeChecked();
  await expect(count).toHaveText(`2 von ${deletable} ausgewählt`);

  await pickBar(page).getByRole("button", { name: /^Löschen …$/ }).click();
  const dialog = page.getByRole("dialog", { name: "2 Sicherungen löschen?" });
  await expect(dialog).toContainText("darunter 2 eigene");
  const [del] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/backups/delete") && r.request().method() === "POST"),
    dialog.getByRole("button", { name: "2 löschen" }).click(),
  ]);
  expect((del.request().postDataJSON() as { names: string[] }).names.sort()).toEqual([a, c].sort());
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("status").filter({ hasText: "Gelöscht: 2 Sicherungen." })).toBeVisible();
  const left = await names();
  expect(left.has(a)).toBe(false);
  expect(left.has(c)).toBe(false);
  expect(left.has(b)).toBe(true);
  // Picking ends; the rows have their buttons again.
  await expect(page.getByRole("button", { name: "Mehrere löschen" })).toBeVisible();
  await expect(manual.getByRole("listitem").first().getByRole("button", { name: /wiederherstellen$/ })).toBeVisible();
});

test("Mehrere löschen: Abbrechen der Rückfrage löscht nichts", async ({ page, open }) => {
  const made = await threeManual();
  await open("/settings/backups");
  await page.getByRole("button", { name: "Mehrere löschen" }).click();
  const boxes = page.getByRole("list", { name: "Manuell" }).getByRole("checkbox");
  await boxes.nth(0).click();
  await boxes.nth(1).click();
  const dialog = page.getByRole("dialog", { name: "2 Sicherungen löschen?" });

  let sent = 0;
  page.on("request", (r) => r.url().endsWith("/api/backups/delete") && sent++);
  await pickBar(page).getByRole("button", { name: /^Löschen …$/ }).click();
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  await expect(dialog).toBeHidden();
  await pickBar(page).getByRole("button", { name: /^Löschen …$/ }).click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  expect(sent).toBe(0);
  const left = await names();
  for (const n of made) expect(left.has(n)).toBe(true);
  // The choice stays for another try.
  await expect(page.locator(".x4-picked")).toHaveText(/^2 von \d+ ausgewählt$/);
});

test("Älter als N Tage: löscht alte automatische, eigene bleiben", async ({ page, open }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA for old backups");
  const fresh = await backupNow();
  const oldAuto = "rp-app-2020-01-03-120000-daily.db";
  const oldOwn = "rp-app-2020-01-04-120000-manual.db";
  placed.push(placeOld(oldAuto, fresh.name), placeOld(oldOwn, fresh.name));
  await open("/settings/backups");
  await expect(page.getByRole("list", { name: "Manuell" }).getByRole("listitem")).not.toHaveCount(0);

  const days = page.getByLabel("Alte automatische Sicherungen löschen", { exact: true });
  await days.fill("0");
  await expect(page.locator("#x4-older-help")).toHaveText("Eine ganze Zahl von 1 bis 3650.");
  await days.fill("30");
  await page.locator('[data-spot="backup-cleanup"]').getByRole("button", { name: /^Löschen …$/ }).click();
  const dialog = page.getByRole("dialog", { name: "Alte automatische Sicherungen löschen?" });
  await expect(dialog).toContainText("1 automatische Sicherung ist älter als 30 Tage");
  await expect(dialog).toContainText("Deine eigenen bleiben.");
  const [del] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/backups/delete") && r.request().method() === "POST"),
    dialog.getByRole("button", { name: "1 löschen" }).click(),
  ]);
  expect(del.request().postDataJSON()).toEqual({ older_than_days: 30 });
  expect(await del.json()).toEqual({ deleted: [oldAuto] });
  await expect(page.getByRole("status").filter({ hasText: "Gelöscht: 1 Sicherung." })).toBeVisible();

  const left = await names();
  expect(left.has(oldAuto)).toBe(false);
  expect(left.has(oldOwn)).toBe(true);
  expect(left.has(fresh.name)).toBe(true);

  // Nothing older any more: it says so instead of asking.
  await page.locator('[data-spot="backup-cleanup"]').getByRole("button", { name: /^Löschen …$/ }).click();
  await expect(page.getByRole("status").filter({ hasText: "Keine automatische Sicherung ist älter als 30 Tage." })).toBeVisible();
  await expect(dialog).toBeHidden();
});

test("Strg K: Grenze, alte und mehrere Sicherungen löschen führen an die Stelle", async ({ page, open }) => {
  await open("/chars");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  for (const [query, label, spot] of [
    ["grenze sicherungen", "Grenze für Sicherungen", "backup-limit"],
    ["alte sicherungen", "Alte Sicherungen löschen", "backup-cleanup"],
    ["mehrere sicherungen", "Mehrere Sicherungen löschen", "backup-pick"],
  ] as const) {
    await page.keyboard.press("Control+k");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("combobox").fill(query);
    await expect(dialog.getByRole("option").first()).toContainText(label);
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect(page).toHaveURL(/#\/settings\/backups$/);
    await expect(page.locator(`[data-spot="${spot}"]`)).toBeInViewport();
    await expect(page.locator(`[data-spot="${spot}"]`)).toHaveClass(/spot-on/);
  }
  // The limit's spot puts the focus on the chosen size.
  await page.keyboard.press("Control+k");
  await dialog.getByRole("combobox").fill("grenze sicherungen");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("radiogroup", { name: "Grenze" }).getByRole("radio", { name: "1 GB" })).toBeFocused();
});
