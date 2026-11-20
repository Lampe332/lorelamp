// The import page: many cards at once (with a broken file, a duplicate and a note that holds no
// card), the persona import of a SillyTavern backup and the link field. Everything the tests add is
// deleted again, so the other screens keep their three test cards and two personas.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { api, fixture, library } from "./api";
import { expect, test } from "./fixtures";

type FilePayload = { name: string; mimeType: string; buffer: Buffer };

/** A small SFW card in the V2 format; `run` keeps the cards of each run apart (duplicates are found by content). */
const card = (run: string, n: number): FilePayload => ({
  name: `importtest-${n}.json`,
  mimeType: "application/json",
  buffer: Buffer.from(
    JSON.stringify({
      spec: "chara_card_v2",
      spec_version: "2.0",
      data: {
        name: `Importtest ${run} ${n}`,
        description: `Eine Testfigur aus dem Massen-Import, Nummer ${n} (Lauf ${run}).`,
        personality: "ruhig",
        scenario: "Ein Bahnhof am Morgen.",
        first_mes: "Guten Morgen.",
        mes_example: "",
        tags: ["Test"],
      },
    }),
  ),
});

test("Import: viele Karten auf einmal mit Ergebnisliste", async ({ page, open }) => {
  const run = Date.now().toString(36);
  const files: FilePayload[] = [
    ...Array.from({ length: 20 }, (_, i) => card(run, i + 1)),
    { name: "kaputt.json", mimeType: "application/json", buffer: Buffer.from("{ kaputt") },
    { name: "mara_voss.json", mimeType: "application/json", buffer: Buffer.from(fixture("mara_voss.json")) },
    { name: "notizen.txt", mimeType: "text/plain", buffer: Buffer.from("Nur eine Notiz, keine Karte.") },
  ];
  try {
    await open("/import");
    await expect(page.getByRole("heading", { name: "Importieren", level: 1 })).toBeVisible();
    const chooser = page.waitForEvent("filechooser");
    await page.getByRole("button", { name: "Dateien wählen", exact: true }).click();
    await (await chooser).setFiles(files);

    const result = page.getByRole("region", { name: "Ergebnis" });
    await expect(result).toBeVisible();
    await expect(result.getByText("23 Dateien gelesen.")).toBeVisible();
    const figure = (label: string) => result.locator(".imp-sum > div").filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
    await expect(figure("Importiert")).toHaveText("20");
    await expect(figure("Schon vorhanden")).toHaveText("1");
    await expect(figure("Fehler")).toHaveText("1");

    const rows = result.getByRole("list", { name: "Dateien" }).getByRole("listitem");
    await expect(rows).toHaveCount(22);
    // Imported cards link to their figure (names arrive with the library).
    await expect(rows.getByRole("link", { name: `Importtest ${run} 1`, exact: true })).toBeVisible();
    await expect(rows.filter({ hasText: "mara_voss.json" })).toContainText("schon in der Bibliothek");

    await result.getByRole("button", { name: /^Fehler/ }).click();
    await expect(rows).toHaveCount(1);
    await expect(rows).toContainText("kaputt.json");
    await expect(rows).toContainText("Beschädigt");

    // The note is no error: it waits folded away.
    const skipped = result.getByText("1 Datei ohne Karte übersprungen");
    await expect(skipped).toBeVisible();
    await skipped.click();
    await expect(result.locator(".imp-skip")).toContainText("notizen.txt");

    await result.getByRole("button", { name: /^Alle/ }).click();
    await rows.getByRole("link", { name: `Importtest ${run} 1`, exact: true }).click();
    await expect(page).toHaveURL(/#\/char\/\d+\/settings$/);
    await expect(page.getByRole("heading", { name: `Importtest ${run} 1`, level: 1 })).toBeVisible();
  } finally {
    for (const c of (await library()).characters.filter((c) => c.name.startsWith(`Importtest ${run} `))) {
      await api("DELETE", `/characters/${c.id}`);
    }
  }
});

test("Import: Personas aus einem SillyTavern-Backup", async ({ page, open }) => {
  const before = new Set((await library()).personas.map((p) => p.id));
  try {
    await open("/import");
    const chooser = page.waitForEvent("filechooser");
    await page.getByRole("button", { name: "Persona-Dateien wählen" }).click();
    await (await chooser).setFiles({
      name: "personas_20261115.json",
      mimeType: "application/json",
      buffer: Buffer.from(fixture("formats/personas_20261115.json")),
    });
    const result = page.getByRole("region", { name: "Ergebnis" });
    await expect(result.locator(".imp-sum > div").first().locator("dd")).toHaveText("2");
    const rows = result.getByRole("list", { name: "Dateien" }).getByRole("listitem");
    await expect(rows).toHaveCount(2);
    for (const name of ["Jonas", "Wanderin"]) {
      await expect(rows.getByRole("link", { name, exact: true })).toHaveAttribute("href", /^#\/personas\/\d+$/);
    }
  } finally {
    for (const p of (await library()).personas.filter((p) => !before.has(p.id))) {
      await api("DELETE", `/personas/${p.id}`);
    }
  }
});

/** A plain 1×1 PNG without card data (a persona picture). */
const PIXEL = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64");

test("Import: SillyTavern-Ordner übernehmen", async ({ page, open }) => {
  const run = Date.now().toString(36);
  const before = new Set((await library()).personas.map((p) => p.id));
  // A small copy of a SillyTavern data folder: two cards, an expression picture in a subfolder and a
  // chat (both stay out), settings.json with one persona and its picture.
  const root = mkdtempSync(join(tmpdir(), "rp-st-"));
  const user = join(root, "default-user");
  for (const dir of ["characters/Sprites", "chats/Leuchtturm", "User Avatars"]) mkdirSync(join(user, dir), { recursive: true });
  for (const n of [1, 2]) writeFileSync(join(user, "characters", `st-${n}.json`), card(`ST ${run}`, n).buffer);
  writeFileSync(join(user, "characters/Sprites", "freude.png"), PIXEL);
  writeFileSync(join(user, "chats/Leuchtturm", "chat.jsonl"), "{}");
  writeFileSync(join(user, "User Avatars", "st-persona.png"), PIXEL);
  writeFileSync(
    join(user, "settings.json"),
    JSON.stringify({
      power_user: {
        personas: { "st-persona.png": `Reisende ${run}` },
        persona_descriptions: { "st-persona.png": { description: "Eine Reisende aus dem Test." } },
      },
    }),
  );
  try {
    await open("/import");
    const chooser = page.waitForEvent("filechooser");
    await page.getByRole("button", { name: "Ordner selbst wählen" }).click();
    await (await chooser).setFiles(user);
    const result = page.getByRole("region", { name: "Ergebnis" });
    await expect(result.locator(".imp-sum > div").first().locator("dd")).toHaveText("3");
    const rows = result.getByRole("list", { name: "Dateien" }).getByRole("listitem");
    await expect(rows).toHaveCount(3);
    await expect(rows.getByRole("link", { name: `Reisende ${run}` })).toHaveAttribute("href", /^#\/personas\/\d+$/);
    await expect(result.locator(".imp-skip")).toHaveCount(0);
    const persona = (await library()).personas.find((p) => p.name === `Reisende ${run}`);
    expect(persona).toBeTruthy();
  } finally {
    rmSync(root, { recursive: true, force: true });
    for (const c of (await library()).characters.filter((c) => c.name.startsWith(`Importtest ST ${run} `))) await api("DELETE", `/characters/${c.id}`);
    for (const p of (await library()).personas.filter((p) => !before.has(p.id))) await api("DELETE", `/personas/${p.id}`);
  }
});

test("Import: Link-Feld", async ({ page, open }) => {
  await open("/import");
  const field = page.getByRole("textbox", { name: "Link einfügen" });
  await field.fill("kein link");
  await page.getByRole("button", { name: "Vorschau", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Das ist kein Link");

  // Ctrl+V on the page puts the link into the field and asks for its preview. A site that allows no
  // direct fetch gets the way by file; a server without the preview route says so plainly. Neither
  // goes out to the network.
  const link = "https://janitorai.com/characters/beispiel-leuchtturm";
  await page.evaluate((text) => {
    const data = new DataTransfer();
    data.setData("text/plain", text);
    document.body.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true }));
  }, link);
  await expect(field).toHaveValue(link);
  const fileOnly = page.locator(".imp-fileonly");
  const noRoute = page.getByRole("alert").filter({ hasText: /^Dieser Server kann keine Links abrufen \(Antwort 40[45]\)/ });
  await expect(fileOnly.or(noRoute)).toBeVisible();
  if (await fileOnly.isVisible()) {
    await expect(fileOnly).toContainText("erlaubt keinen direkten Abruf");
    await expect(fileOnly.getByRole("link", { name: "Seite öffnen" })).toHaveAttribute("href", /janitorai\.com/);
  }
});

test("Import: erreichbar von den Charakteren und über Strg+K", async ({ page, open }) => {
  await open("/chars");
  await page.getByRole("link", { name: "Karte importieren" }).click();
  await expect(page).toHaveURL(/#\/import$/);
  await open("/");
  await page.keyboard.press("Control+k");
  await page.keyboard.type("sillytavern");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/import$/);
  await expect(page.getByRole("heading", { name: "Aus SillyTavern übernehmen", level: 2 })).toBeVisible();
});
