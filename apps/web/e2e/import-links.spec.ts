// The link import's general way and everything Ctrl+V takes on the import page: a link of any
// site, several links at once, a pasted card picture, a pasted picture without card data, a card
// copied as text and a picture copied on a web page. The links go through the real server to a
// local stand-in site (`card-site-standin.ts`); without `RP_E2E_LINK_SITE` those tests skip
// themselves. Everything the tests add is removed again.
//
// `RP_E2E_SHOTS` (optional): review screenshots go there.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Server } from "node:http";
import type { Page, TestInfo } from "@playwright/test";
import { api, library } from "./api";
import { cardJson, LINK_SITE, png, startCardSite } from "./card-site-standin";
import { expect, isPhone, test } from "./fixtures";

const shots = process.env.RP_E2E_SHOTS;

async function shot(page: Page, info: TestInfo, name: string, scrollTo?: string) {
  if (!shots) return;
  if (scrollTo) await page.locator(scrollTo).first().evaluate((el) => el.scrollIntoView({ block: "start" }));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(350); // the preview's short rise
  mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: join(shots, `${name}-${info.project.name}.png`) });
}

/** Ctrl+V on the page: text, HTML and files as a browser puts them on the clipboard. */
const pasteOnPage = (page: Page, what: { text?: string; html?: string; files?: { name: string; type: string; base64: string }[] }) =>
  page.evaluate((w) => {
    const data = new DataTransfer();
    if (w.text) data.setData("text/plain", w.text);
    if (w.html) data.setData("text/html", w.html);
    for (const f of w.files ?? []) {
      const bytes = Uint8Array.from(atob(f.base64), (c) => c.charCodeAt(0));
      data.items.add(new File([bytes], f.name, { type: f.type }));
    }
    document.body.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
  }, what);

const run = () => Date.now().toString(36);
const named = async (prefix: string) => (await library()).characters.filter((c) => c.name.startsWith(prefix));
const removeNamed = async (prefix: string) => {
  for (const c of await named(prefix)) await api("DELETE", `/characters/${c.id}`);
};

let site: { server: Server; seen: string[] } | null = null;
test.beforeAll(async () => {
  if (LINK_SITE) site = await startCardSite();
});
test.afterAll(async () => {
  await new Promise<void>((resolve) => (site ? site.server.close(() => resolve()) : resolve()));
});

test("Link-Import: der Link einer beliebigen Seite zeigt die Vorschau, ein Klick importiert", async ({ page, open }, info) => {
  test.skip(!LINK_SITE, "needs the stand-in site (RP_E2E_LINK_SITE)");
  const name = `Bahnführerin ${run()} ${info.project.name}`;
  const link = `${LINK_SITE}/page/${encodeURIComponent(name)}`;
  try {
    await open("/import");
    await pasteOnPage(page, { text: link });
    const preview = page.getByRole("article", { name });
    await expect(preview).toBeVisible();
    await expect(page.getByRole("textbox", { name: "Link einfügen" })).toHaveValue(link);
    await expect(preview.getByText("rp-app-tests")).toBeVisible();
    await expect(preview.getByRole("list", { name: "Tags" }).getByRole("listitem")).toHaveCount(5);
    await expect(preview.getByText(`${name} führt die kleine Bergbahn`)).toBeVisible();
    await expect(preview.getByRole("link", { name: "auf 127.0.0.1" })).toHaveAttribute("href", link);
    // The page's own picture became the card's.
    await expect(preview.locator("img.imp-pv-pic")).toHaveJSProperty("complete", true);
    expect(await preview.locator("img.imp-pv-pic").evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);
    expect(await named(name)).toHaveLength(0);
    await shot(page, info, "link-vorschau", ".imp-link");

    await preview.getByRole("button", { name: "Importieren" }).click();
    const done = page.locator(".imp-link-msg").filter({ hasText: "Importiert" });
    await expect(done.getByRole("link", { name })).toHaveAttribute("href", /^#\/char\/\d+\/settings$/);
    await expect(page.getByRole("textbox", { name: "Link einfügen" })).toHaveValue("");
    const [made] = await named(name);
    expect(made?.has_avatar).toBe(true);

    // A page that only links its card file: found the same way, and known as the same card.
    await page.getByRole("textbox", { name: "Link einfügen" }).fill(`${LINK_SITE}/card/${encodeURIComponent(name)}.json`);
    await page.getByRole("textbox", { name: "Link einfügen" }).press("Enter");
    await expect(page.getByRole("article", { name }).locator(".imp-pv-dup")).toContainText("Schon in deiner Bibliothek");
  } finally {
    await removeNamed(name);
  }
});

test("Link-Import: mehrere Links auf einmal, jede Seite bekommt ihren Weg", async ({ page, open }, info) => {
  test.skip(!LINK_SITE, "needs the stand-in site (RP_E2E_LINK_SITE)");
  const prefix = `Sammellink ${run()} ${info.project.name}`;
  const names = [`${prefix} A`, `${prefix} B`, `${prefix} C`];
  const pasted = [
    `${LINK_SITE}/card/${encodeURIComponent(names[0])}.json`,
    `${LINK_SITE}/linked/${encodeURIComponent(names[1])}`,
    `${LINK_SITE}/blocked`,
    "https://janitorai.com/characters/beispiel-bergbahn",
    `${LINK_SITE}/page/${encodeURIComponent(names[2])}`,
    `${LINK_SITE}/card/${encodeURIComponent(names[0])}.json`, // twice: taken once
    `${LINK_SITE}/empty`,
  ].join("\n");
  try {
    await open("/import");
    await pasteOnPage(page, { text: pasted });
    const list = page.getByRole("list", { name: "Eingefügte Links" });
    await expect(list.locator(":scope > li")).toHaveCount(6);
    await expect(page.locator(".lnk-count")).toContainText("6 Links");
    await expect(page.locator(".lnk-count")).toContainText("alle geholt");
    // Several links stay out of the field.
    await expect(page.getByRole("textbox", { name: "Link einfügen" })).toHaveValue("");
    for (const name of names) await expect(list.getByRole("article", { name })).toBeVisible();

    // A bot check and a site that forbids programs: the way through the user, nothing worked around.
    const blocked = list.locator(".imp-fileonly").filter({ hasText: "127.0.0.1 erlaubt keinen direkten Abruf" });
    await expect(blocked).toContainText("lässt nur Browser durch");
    await expect(blocked.getByRole("link", { name: "Seite öffnen" })).toHaveAttribute("href", `${LINK_SITE}/blocked`);
    await expect(blocked.getByRole("button", { name: "Datei wählen" })).toBeVisible();
    const janitor = list.locator(".imp-fileonly").filter({ hasText: "JanitorAI erlaubt keinen direkten Abruf" });
    await expect(janitor.getByRole("link", { name: "Seite öffnen" })).toHaveAttribute("href", /janitorai\.com/);
    await expect(list.locator(".imp-fileonly").filter({ hasText: "hat die App keine Karte gefunden" })).toBeVisible();
    expect(site?.seen.filter((p) => p === "/blocked")).toHaveLength(1);
    expect(await named(prefix)).toHaveLength(0);
    await shot(page, info, "links-liste", ".imp-link");

    // On a phone every control is a full touch target.
    if (isPhone(page)) {
      for (const button of await list.getByRole("button").all()) expect((await button.boundingBox())?.height ?? 0).toBeGreaterThanOrEqual(44);
    }

    // One of them is discarded, the rest comes in with one click.
    await list.getByRole("article", { name: names[1] }).getByRole("button", { name: "Verwerfen" }).click();
    await expect(list.locator(":scope > li")).toHaveCount(5);
    await page.getByRole("button", { name: "Alle 2 importieren" }).click();
    await expect(list.locator(".imp-link-msg").filter({ hasText: "Importiert" })).toHaveCount(2);
    await expect(list.getByRole("link", { name: names[0] })).toHaveAttribute("href", /^#\/char\/\d+\/settings$/);
    expect((await named(prefix)).map((c) => c.name).sort()).toEqual([names[0], names[2]]);
    await shot(page, info, "links-importiert", ".imp-link");

    await page.getByRole("button", { name: "Liste leeren" }).click();
    await expect(list).toHaveCount(0);
    await expect(page.locator(".imp-fileonly")).toHaveCount(0);
  } finally {
    await removeNamed(prefix);
  }
});

test("Link-Import: eine lokale Adresse wird nicht abgerufen", async ({ page, open }) => {
  await open("/import");
  const field = page.getByRole("textbox", { name: "Link einfügen" });
  await field.fill("https://192.168.0.1/karte.png");
  await field.press("Enter");
  await expect(page.getByRole("alert")).toContainText("Diese Adresse ruft die App nicht ab");
});

test("Einfügen: ein Karten-Bild kommt als Karte an, ein kopiertes Bild ohne Daten wird erklärt", async ({ page, open }, info) => {
  const name = `Bildkarte ${run()} ${info.project.name}`;
  try {
    await open("/import");
    await pasteOnPage(page, { files: [{ name: "karte.png", type: "image/png", base64: png(cardJson(name)).toString("base64") }] });
    const result = page.getByRole("region", { name: "Ergebnis" });
    await expect(result.getByRole("list", { name: "Dateien" }).getByRole("link", { name, exact: true })).toHaveAttribute("href", /^#\/char\/\d+\/settings$/);
    const [made] = await named(name);
    expect(made?.has_avatar).toBe(true);

    // What "Bild kopieren" leaves on the clipboard: pixels only.
    await result.getByRole("button", { name: "Liste leeren" }).click();
    await pasteOnPage(page, { files: [{ name: "image.png", type: "image/png", base64: png().toString("base64") }] });
    await expect(page.getByRole("alert").filter({ hasText: "Das eingefügte Bild enthält keine Kartendaten" })).toBeVisible();
    await shot(page, info, "bild-ohne-karte", ".imp-drop");
  } finally {
    await removeNamed(name);
  }
});

test("Einfügen: eine als Text kopierte Karte wird importiert, anderer Text nicht", async ({ page, open }, info) => {
  const name = `Textkarte ${run()} ${info.project.name}`;
  try {
    await open("/import");
    await pasteOnPage(page, { text: "nur ein Satz ohne Link" });
    await expect(page.getByRole("alert")).toContainText("weder ein Link noch eine Karte");
    await pasteOnPage(page, { text: cardJson(name) });
    const result = page.getByRole("region", { name: "Ergebnis" });
    await expect(result.getByRole("list", { name: "Dateien" }).getByRole("link", { name, exact: true })).toBeVisible();
    expect(await named(name)).toHaveLength(1);
  } finally {
    await removeNamed(name);
  }
});

test("Einfügen: ein auf einer Webseite kopiertes Bild wird über seine Adresse geholt", async ({ page, open }, info) => {
  test.skip(!LINK_SITE, "needs the stand-in site (RP_E2E_LINK_SITE)");
  const name = `Kopiertes Bild ${run()} ${info.project.name}`;
  const address = `${LINK_SITE}/card/${encodeURIComponent(name)}.png`;
  try {
    await open("/import");
    // The browser puts the bare pixels and `<img src>` on the clipboard; the file behind the address
    // still carries the card.
    await pasteOnPage(page, {
      html: `<meta charset="utf-8"><img src="${address.replace(/&/g, "&amp;")}" alt="">`,
      files: [{ name: "image.png", type: "image/png", base64: png().toString("base64") }],
    });
    const preview = page.getByRole("article", { name });
    await expect(preview).toBeVisible();
    await preview.getByRole("button", { name: "Importieren" }).click();
    await expect(page.locator(".imp-link-msg").filter({ hasText: "Importiert" }).getByRole("link", { name })).toBeVisible();
    expect((await named(name))[0]?.has_avatar).toBe(true);
  } finally {
    await removeNamed(name);
  }
});
