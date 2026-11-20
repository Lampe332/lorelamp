// Settings › Sicherungen: the overview (last backup, room used against the limit, what is kept), the
// backups grouped by kind, back up now, delete with a question, a copy made by hand (no delete), the
// hint when a manual backup would not fit, and restore. Everything a test adds (a character, manual
// backups, the copy made by hand) is removed again, so the other tests see the data as it was.

import { copyFileSync, rmSync, utimesSync } from "node:fs";
import { join } from "node:path";
import { api, library } from "./api";
import { dataDir } from "./db";
import { expect, test } from "./fixtures";

const NAME = "Sicherungstest";
const HAND = "rp-app.db.backup-e2e-hand";

type Backup = { name: string; reason: string; kind: string };
type Overview = { backups: Backup[]; total_bytes: number; limit_bytes: number };

const findId = async () => (await library()).characters.find((c) => c.name === NAME)?.id;
const overview = () => api<Overview>("GET", "/backups");
const manualNames = async () => new Set((await overview()).backups.filter((b) => b.reason === "manual").map((b) => b.name));

let before = new Set<string>();
test.beforeEach(async () => {
  before = await manualNames();
});

test.afterEach(async () => {
  const id = await findId();
  if (id !== undefined) await api("DELETE", `/characters/${id}`);
  // Manual backups are never removed on their own: take back the ones this test made.
  for (const name of await manualNames()) if (!before.has(name)) await api("DELETE", `/backups/${encodeURIComponent(name)}`);
});

test("Sicherungen: Übersicht, Gruppen, Jetzt sichern, Löschen mit Rückfrage, Kopie von Hand", async ({ page, open }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA for the copy made by hand");
  // A copy made by hand next to the database, as the server recognises it (`rp-app.db.backup-…`).
  const hand = join(dataDir, HAND);
  copyFileSync(join(dataDir, "rp-app.db"), hand);
  const month = new Date(Date.now() - 30 * 86_400_000);
  utimesSync(hand, month, month);
  try {
    await open("/settings/backups");
    const tabs = page.getByRole("navigation", { name: "Bereiche der Einstellungen" });
    await expect(tabs.getByRole("link", { name: "Sicherungen" })).toHaveAttribute("aria-current", "page");

    // Overview: the last backup, the room all backups take against the 1 GB limit, what is kept.
    await expect(page.getByRole("heading", { name: "Sichern", level: 2 })).toBeVisible();
    await expect(page.getByText("Letzte Sicherung")).toBeVisible();
    const meter = page.getByRole("meter", { name: "Platz für Sicherungen" });
    await expect(meter).toBeVisible();
    await expect(meter).toHaveAttribute("aria-valuemax", String(1024 ** 3));
    await expect(page.locator(".bk-room-used")).toHaveText(/^\d+(,\d)? (KB|MB|GB) von 1 GB$/);
    await expect(page.getByText(/^Von selbst behalten werden 7 tägliche und 4 wöchentliche Sicherungen/)).toBeVisible();

    // Groups: the copy made by hand is a manual one; the server makes a daily one when it starts.
    await expect(page.getByRole("heading", { name: "Manuell", level: 3 })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Täglich", level: 3 })).toBeVisible();
    const manual = page.getByRole("list", { name: "Manuell" });
    const handRow = manual.getByRole("listitem").filter({ hasText: "Kopie von Hand" });
    await expect(handRow).toHaveCount(1);
    await expect(handRow.getByRole("button", { name: /wiederherstellen$/ })).toBeVisible();
    await expect(handRow.getByRole("button", { name: /löschen$/ })).toHaveCount(0);

    // Back up now: one more manual backup, newest first in its group.
    const count = await manual.getByRole("listitem").count();
    const [made] = await Promise.all([
      page.waitForResponse((r) => r.url().endsWith("/api/backups") && r.request().method() === "POST"),
      page.getByRole("button", { name: "Jetzt sichern" }).click(),
    ]);
    const { name } = (await made.json()) as Backup;
    await expect(page.getByRole("status").filter({ hasText: /^Gesichert: heute, \d\d:\d\d/ })).toBeVisible();
    await expect(manual.getByRole("listitem")).toHaveCount(count + 1);
    const newest = manual.getByRole("listitem").first();
    await expect(newest).toContainText("heute");

    // Delete asks first; "Abbrechen" keeps it.
    await newest.getByRole("button", { name: /löschen$/ }).click();
    const dialog = page.getByRole("dialog", { name: "Diese Sicherung löschen?" });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText("Das kann nicht rückgängig gemacht werden.");
    await dialog.getByRole("button", { name: "Abbrechen" }).click();
    await expect(dialog).toBeHidden();
    expect((await overview()).backups.some((b) => b.name === name)).toBe(true);

    await newest.getByRole("button", { name: /löschen$/ }).click();
    await dialog.getByRole("button", { name: "Löschen" }).click();
    await expect(dialog).toBeHidden();
    await expect(page.getByRole("status").filter({ hasText: /^Gelöscht: die Sicherung von / })).toBeVisible();
    await expect(manual.getByRole("listitem")).toHaveCount(count);
    expect((await overview()).backups.some((b) => b.name === name)).toBe(false);
  } finally {
    rmSync(hand, { force: true });
  }
});

test("Sicherungen: ist kein Platz mehr, sagt die Seite, was zu tun ist", async ({ page, open }) => {
  // A real 409 needs 1 GB of manual backups; only this one answer is answered by the test.
  await page.route("**/api/backups", (route) =>
    route.request().method() === "POST" ? route.fulfill({ status: 409, contentType: "application/json", body: '{"error":"backup_limit"}' }) : route.continue(),
  );
  await open("/settings/backups");
  await page.getByRole("button", { name: "Jetzt sichern" }).click();
  const hint = page.getByRole("alert").filter({ hasText: "Kein Platz für eine weitere eigene Sicherung" });
  await expect(hint).toBeVisible();
  await expect(hint).toContainText("höchstens 1 GB");
  await expect(hint).toContainText("Lösche unten unter „Manuell“ eine ältere");
});

test("Sicherungen: sichern, Figur löschen, wiederherstellen", async ({ page, open }) => {
  await api("POST", "/characters/import", new TextEncoder().encode(JSON.stringify({ name: NAME, description: "Nur für den Test." })));
  const id = await findId();
  expect(id).toBeDefined();

  await open("/settings/backups");
  await page.getByRole("button", { name: "Jetzt sichern" }).click();
  await expect(page.getByRole("status").filter({ hasText: /^Gesichert: heute, \d\d:\d\d/ })).toBeVisible();
  const newest = page.getByRole("list", { name: "Manuell" }).getByRole("listitem").first();
  await expect(newest).toContainText("heute");

  // After the backup the character is deleted; the next start would purge it for good.
  await api("DELETE", `/characters/${id}`);
  expect(await findId()).toBeUndefined();

  await newest.getByRole("button", { name: /wiederherstellen$/ }).click();
  const dialog = page.getByRole("dialog", { name: "Diesen Stand wiederherstellen?" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("Der aktuelle Stand wird vorher gesichert");
  // Cancel changes nothing.
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  await expect(dialog).toBeHidden();
  expect(await findId()).toBeUndefined();

  await newest.getByRole("button", { name: /wiederherstellen$/ }).click();
  await dialog.getByRole("button", { name: "Wiederherstellen" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("status").filter({ hasText: /^Wiederhergestellt: der Stand von heute/ })).toBeVisible();
  await expect(page.getByRole("list", { name: "Vor Wiederherstellen" }).getByRole("listitem").first()).toContainText("heute");
  expect(await findId()).toBe(id);

  await open("/chars");
  await expect(page.getByRole("button", { name: `${NAME}: Aktionen zeigen` })).toBeVisible();
});

test("Strg K: „sicherung“, „backup“, „jetzt sichern“ und „wiederherstellen“ führen zu den Sicherungen", async ({ page, open }) => {
  await open("/chars");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  for (const [query, label, spot] of [
    ["sicherung", "Sicherung", "backups"],
    ["backup", "Sicherung", "backups"],
    ["jetzt sichern", "Jetzt sichern", "backup-now"],
    ["wiederherstellen", "Sicherung wiederherstellen", "backups"],
  ] as const) {
    await page.keyboard.press("Control+k");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("combobox").fill(query);
    await expect(dialog.getByRole("option").first()).toContainText(label);
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect(page).toHaveURL(/#\/settings\/backups$/);
    await expect(page.locator(`[data-spot="${spot}"]`)).toBeVisible();
  }
});
