// SillyTavern import with a choice (X6): "Übernehmen" opens a dialog with every card, persona and chat
// of the installation; only "Importieren" takes over exactly what is ticked.
//
// Like `import-preview.spec.ts`, the test server is started with `RP_ST_SEARCH_ROOTS=<folder>` (never
// the real drives) and the tests get the same folder in `RP_E2E_ST_ROOT`; they build a small
// SillyTavern there (the SFW fixtures of `tests/fixtures/sillytavern` plus two invented
// characters) and remove it again. Without it they skip themselves.
//
// `RP_E2E_SHOTS` (optional): review screenshots of the open dialog and of its result go there.

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page, TestInfo } from "@playwright/test";
import { api, fixture, library } from "./api";
import { expect, test } from "./fixtures";

const stRoot = process.env.RP_E2E_ST_ROOT;
const shots = process.env.RP_E2E_SHOTS;

async function shot(page: Page, info: TestInfo, name: string) {
  if (!shots) return;
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(400); // the dialog's short rise, the pictures
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, `${name}-${info.project.name}.png`) });
}

/** A small SFW card of an invented character in the V2 format. */
const cardJson = (name: string, job: string) =>
  JSON.stringify({
    spec: "chara_card_v2",
    spec_version: "2.0",
    data: {
      name,
      description: `{{char}} ist 38 Jahre alt und ${job}. Sie erklärt gern, was sie tut, und hat immer eine Thermoskanne Tee dabei.`,
      personality: "ruhig, genau, freundlich",
      scenario: "Ein gewöhnlicher Arbeitstag.",
      first_mes: "Guten Morgen. Schön, dass du da bist.",
      mes_example: "",
      creator: "rp-app-tests",
      tags: ["Alltag"],
    },
  });

/** A plain 1×1 PNG without card data (a persona picture). */
const PIXEL = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64");

const THEO_CHAT = "Theo Lindqvist - 2026-03-01@18h30m00s";
const GRETA_CHAT = "Greta Holm - 2026-04-02@10h00m00s";
const FIXED = ["Theo Lindqvist", "Greta Holm"];

/** Four cards (two of them with a chat), a chat without a card and one persona. */
function buildInstall(run: string) {
  const install = join(stRoot!, `SillyTavern-x6-${run}`);
  const user = join(install, "data", "default-user");
  for (const dir of ["characters", "chats/theo_lindqvist", "chats/greta_holm", "chats/Niemand", "User Avatars"]) mkdirSync(join(user, dir), { recursive: true });
  writeFileSync(join(user, "characters", "theo_lindqvist.json"), fixture("sillytavern/theo_lindqvist.json"));
  writeFileSync(join(user, "characters", "greta_holm.json"), fixture("sillytavern/greta_holm.json"));
  writeFileSync(join(user, "characters", "ada_brenner.json"), cardJson(`Ada Brenner ${run}`, "Glasbläserin in einer kleinen Werkstatt am Hafen"));
  writeFileSync(join(user, "characters", "jorin_falk.json"), cardJson(`Jorin Falk ${run}`, "Försterin in einem Mischwald an der Weser"));
  writeFileSync(join(user, "chats/theo_lindqvist", `${THEO_CHAT}.jsonl`), fixture("sillytavern/chat_theo.jsonl"));
  writeFileSync(join(user, "chats/greta_holm", `${GRETA_CHAT}.jsonl`), fixture("sillytavern/chat_greta.jsonl"));
  writeFileSync(join(user, "chats/Niemand", "Niemand - 2026-05-03@08h00m00s.jsonl"), fixture("sillytavern/chat_orphan.jsonl"));
  writeFileSync(join(user, "User Avatars", "x6-persona.png"), PIXEL);
  writeFileSync(
    join(user, "settings.json"),
    JSON.stringify({ power_user: { personas: { "x6-persona.png": `Wanderin ${run}` }, persona_descriptions: { "x6-persona.png": { description: "Eine Wanderin aus dem Test." } } } }),
  );
  return install;
}

const ours = (run: string) => (c: { name: string }) => FIXED.includes(c.name) || c.name.endsWith(` ${run}`);

async function cleanUp(install: string, run: string, personasBefore: Set<number>) {
  rmSync(install, { recursive: true, force: true });
  const lib = await library();
  for (const c of lib.characters.filter(ours(run))) await api("DELETE", `/characters/${c.id}`);
  for (const p of lib.personas.filter((p) => !personasBefore.has(p.id))) await api("DELETE", `/personas/${p.id}`);
  await api("GET", "/import/sillytavern/find?refresh=1");
}

/** Prepares the installation and opens the dialog of its "Übernehmen". */
async function openDialog(page: Page, open: (hash: string) => Promise<void>, run: string) {
  await open("/import");
  const row = page.getByRole("list", { name: "SillyTavern gefunden" }).getByRole("listitem").filter({ hasText: `SillyTavern-x6-${run}` });
  await row.getByRole("button", { name: "Übernehmen" }).click();
  const dialog = page.getByRole("dialog", { name: "Aus SillyTavern übernehmen" });
  await expect(dialog.getByRole("heading", { name: "Karten" })).toBeVisible();
  return dialog;
}

test.describe("SillyTavern mit Auswahl", () => {
  test.skip(!stRoot, "needs RP_ST_SEARCH_ROOTS on the test server and the same folder in RP_E2E_ST_ROOT");

  let run = "";
  let install = "";
  let personasBefore = new Set<number>();
  test.beforeEach(async () => {
    run = Date.now().toString(36);
    // Left over from an aborted run: the fixed test cards must come in as new.
    for (const c of (await library()).characters.filter((c) => FIXED.includes(c.name))) await api("DELETE", `/characters/${c.id}`);
    install = buildInstall(run);
    personasBefore = new Set((await library()).personas.map((p) => p.id));
    await api("GET", "/import/sillytavern/find?refresh=1");
  });
  test.afterEach(async () => {
    if (install) await cleanUp(install, run, personasBefore);
  });

  test("Abbrechen importiert nichts", async ({ page, open }) => {
    const before = (await library()).characters.length;
    const dialog = await openDialog(page, open, run);
    // Everything new is ticked; nothing has been stored by opening.
    for (const name of ["Theo Lindqvist", "Greta Holm", `Ada Brenner ${run}`, `Jorin Falk ${run}`]) await expect(dialog.getByRole("checkbox", { name: `${name} übernehmen` })).toBeChecked();
    await expect(dialog.locator(".x6-total")).toHaveText("4 Karten · 1 Persona · 2 Chats gewählt");
    await dialog.getByRole("button", { name: "Abbrechen" }).click();
    await expect(dialog).toHaveCount(0);
    expect((await library()).characters.length).toBe(before);
    expect((await library()).characters.filter(ours(run))).toHaveLength(0);
    expect((await library()).personas.length).toBe(personasBefore.size);
  });

  test("2 von 4 Karten mit einem Chat, beim zweiten Öffnen sind sie abgewählt", async ({ page, open }, info) => {
    const dialog = await openDialog(page, open, run);
    const card = (name: string) => dialog.getByRole("checkbox", { name: `${name} übernehmen` });
    const chat = (name: string) => dialog.getByRole("checkbox", { name: `Chat ${name} übernehmen` });
    await expect(dialog.getByRole("group", { name: "Chats mit Theo Lindqvist" })).toContainText("5 Nachrichten");
    // The chat without a card is listed, but cannot be chosen.
    const orphan = dialog.getByRole("group", { name: "Ohne passende Karte" });
    await expect(orphan).toContainText("Keine Karte in SillyTavern passt zu diesem Chat");
    await expect(orphan.getByRole("checkbox")).toHaveCount(0);
    await shot(page, info, "x6-dialog");

    await dialog.getByRole("button", { name: "Keine Karten wählen" }).click();
    // Without their card, the chats cannot be chosen any more.
    await expect(chat(THEO_CHAT)).toBeDisabled();
    await expect(chat(GRETA_CHAT)).toBeDisabled();
    await card("Theo Lindqvist").check();
    await card(`Ada Brenner ${run}`).check();
    await expect(chat(THEO_CHAT)).toBeEnabled();
    await expect(chat(THEO_CHAT)).toBeChecked();
    await expect(chat(GRETA_CHAT)).toBeDisabled();
    await expect(dialog.getByRole("group", { name: "Chats mit Greta Holm" })).toContainText("Erst die Karte wählen");
    // Personas are chosen on their own.
    await dialog.getByRole("button", { name: "Keine Personas wählen" }).click();
    await expect(dialog.locator(".x6-total")).toHaveText("2 Karten · 1 Chat gewählt");

    await dialog.getByRole("button", { name: "Importieren" }).click();
    const sum = (label: string) => dialog.locator(".x6-sum > div").filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
    await expect(sum("Übernommen")).toHaveText("3");
    await expect(sum("Übersprungen")).toHaveText("0");
    await expect(sum("Fehlgeschlagen")).toHaveText("0");
    await expect(dialog.getByRole("link", { name: `Ada Brenner ${run}` })).toHaveAttribute("href", /^#\/char\/\d+\/settings$/);
    const chatLink = dialog.getByRole("link", { name: THEO_CHAT });
    await expect(chatLink).toHaveAttribute("href", /^#\/chat\/\d+$/);
    await shot(page, info, "x6-ergebnis");

    // Exactly the two chosen cards are in the library, no persona came along.
    const lib = await library();
    expect(lib.characters.filter(ours(run)).map((c) => c.name).sort()).toEqual([`Ada Brenner ${run}`, "Theo Lindqvist"]);
    expect(lib.characters.find((c) => c.name === "Theo Lindqvist")?.chats).toBe(1);
    expect(lib.personas.length).toBe(personasBefore.size);

    // The chat is in the app with all its messages.
    const href = (await chatLink.getAttribute("href"))!;
    await dialog.getByRole("button", { name: "Fertig" }).click();
    await expect(dialog).toHaveCount(0);
    await open(href);
    const said = ["Willkommen an Bord.", "Wohin fahren wir heute?", "Rüber nach Amrum, wenn der Wind mitspielt.", "Klingt gut.", "Dann halt dich fest."];
    await expect(page.locator(".msg")).toHaveCount(said.length);
    for (const text of said) await expect(page.locator(".msg").filter({ hasText: text })).toHaveCount(1);

    // Opened again: what came in is now a duplicate and unticked; the rest stays ticked.
    const again = await openDialog(page, open, run);
    await expect(again.getByRole("checkbox", { name: "Theo Lindqvist übernehmen" })).not.toBeChecked();
    await expect(again.getByRole("checkbox", { name: `Ada Brenner ${run} übernehmen` })).not.toBeChecked();
    await expect(again.getByRole("checkbox", { name: "Greta Holm übernehmen" })).toBeChecked();
    await expect(again.getByRole("checkbox", { name: `Jorin Falk ${run} übernehmen` })).toBeChecked();
    await expect(again.locator(".x6-tile").filter({ hasText: "Theo Lindqvist" })).toContainText("Schon in der App");
    // The imported chat: unticked, but choosable (its character is in the app).
    await expect(again.getByRole("checkbox", { name: `Chat ${THEO_CHAT} übernehmen` })).not.toBeChecked();
    await expect(again.getByRole("checkbox", { name: `Chat ${THEO_CHAT} übernehmen` })).toBeEnabled();
    await expect(again.getByRole("group", { name: "Chats mit Theo Lindqvist" })).toContainText("schon in der App");
    await shot(page, info, "x6-doppelt");
    await again.getByRole("button", { name: "Abbrechen" }).click();
    await expect(again).toHaveCount(0);
  });

  test("Ausgeblendete Zeilen eines Chats: nicht übernommen, im Ergebnis genannt", async ({ page, open }, info) => {
    // Greta's chat with two lines SillyTavern hid (`is_system`) between her two messages.
    const head = { user_name: "Jonas", character_name: "Greta Holm", create_date: "2026-04-02@10h00m00s", chat_metadata: {} };
    const lines = [
      head,
      { name: "Greta Holm", is_user: false, send_date: "2026-04-02T09:00:00.000Z", mes: "Vorsicht, nicht zu nah an die Beuten." },
      { name: "Greta Holm", is_user: false, is_system: true, send_date: "2026-04-02T09:00:30.000Z", mes: "Ausgeblendete Notiz eins." },
      { name: "Jonas", is_user: true, is_system: true, send_date: "2026-04-02T09:00:40.000Z", mes: "Ausgeblendete Notiz zwei." },
      { name: "Jonas", is_user: true, send_date: "2026-04-02T09:01:00.000Z", mes: "Ich hätte gern ein Glas Rapshonig." },
    ];
    writeFileSync(join(install, "data", "default-user", "chats/greta_holm", `${GRETA_CHAT}.jsonl`), lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
    await api("GET", "/import/sillytavern/find?refresh=1");
    const dialog = await openDialog(page, open, run);
    await dialog.getByRole("button", { name: "Keine Karten wählen" }).click();
    await dialog.getByRole("button", { name: "Keine Personas wählen" }).click();
    await dialog.getByRole("checkbox", { name: "Greta Holm übernehmen" }).check();
    await expect(dialog.getByRole("checkbox", { name: `Chat ${GRETA_CHAT} übernehmen` })).toBeChecked();
    await expect(dialog.locator(".x6-total")).toHaveText("1 Karte · 1 Chat gewählt");
    await dialog.getByRole("button", { name: "Importieren" }).click();

    const row = dialog.locator(".imp-row.is-imported").filter({ has: page.getByRole("link", { name: GRETA_CHAT }) });
    await expect(row.locator(".imp-sub")).toHaveText("Chat mit Greta Holm");
    await expect(row.locator(".x8-hidden")).toHaveText("2 ausgeblendete Zeilen nicht übernommen");
    // Readable in full (it wraps, never cut), and only on the chat's row.
    expect(await row.locator(".x8-hidden").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    await expect(dialog.locator(".x8-hidden")).toHaveCount(1);
    await shot(page, info, "x8-st-ausgeblendet");

    const href = (await row.getByRole("link").getAttribute("href"))!;
    await dialog.getByRole("button", { name: "Fertig" }).click();
    await open(href);
    await expect(page.locator(".msg")).toHaveCount(2);
    await expect(page.locator(".msg").filter({ hasText: "Ausgeblendete Notiz" })).toHaveCount(0);
  });
});
