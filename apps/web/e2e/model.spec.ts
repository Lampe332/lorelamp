// The model: the sidebar (phone: top bar) names it with quant and context; the settings tab "Modell"
// shows it on top ("Dein Modell"); the tab "Experte" lists the profiles, creates, duplicates, renames
// and deletes one, edits every sampler value with slider, field and "Standard", writes start options
// into the load arguments while unknown arguments stay, discards a draft, switches between two CPU
// profiles with visible progress (only when the test server manages the model itself:
// `--manage-engine`, see README), and Ctrl+K finds the places. "Meine Modelle" and the start options
// that restart the model: models-local.spec.ts.

import { api } from "./api";
import { expect, isPhone, modelStatus, snap, test } from "./fixtures";
import type { Locator, Page } from "@playwright/test";

type Profile = { id: number; name: string; engine_id: number; model_file: string; load_args: string[]; sampler: Record<string, number>; reasoning: string; active: boolean };
type Engine = { status: string; managed: boolean; model: { name: string | null; file: string | null; quant: string | null; ctx: number | null } };

const PREFIX = "E2E ";
const profiles = async () => (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles;
const profile = async (id: number) => (await profiles()).find((p) => p.id === id)!;
const engine = () => api<Engine>("GET", "/engine");
const ctxShort = (n: number) => `${Math.round(n / 1024)}k`;

async function activeProfile(): Promise<Profile> {
  const p = (await profiles()).find((x) => x.active);
  if (!p) throw new Error("no active model profile on the test server");
  return p;
}

/** Waits until the model server answers with the given context (after a switch). */
async function readyWith(ctx: number) {
  await expect
    .poll(async () => {
      const e = await engine();
      return e.status === "ready" ? e.model.ctx : null;
    }, { timeout: 120_000, intervals: [500, 1000, 2000] })
    .toBe(ctx);
}

let original: Profile;

test.beforeAll(async () => {
  original = await activeProfile();
});

// Back to the profile the suite started with, and no test profile left behind.
test.afterEach(async () => {
  const now = await activeProfile();
  if (now.id !== original.id) {
    await api("POST", `/models/profiles/${original.id}/activate`);
    const ctx = ctxOf(original.load_args);
    if ((await engine()).managed && ctx) await readyWith(ctx);
  }
  for (const p of await profiles()) if (p.name.startsWith(PREFIX)) await api("DELETE", `/models/profiles/${p.id}`);
});

function ctxOf(args: string[]): number | null {
  const i = args.findIndex((a) => a === "-c" || a === "--ctx-size");
  return i >= 0 ? Number(args[i + 1]) : null;
}

const row = (page: Page, name: string): Locator => page.getByRole("listitem", { name: `Profil ${name}`, exact: true });
const bar = (page: Page) => page.getByRole("group", { name: "Speichern" });
/** A test profile like the active one, not active (so saving it never restarts the model). */
async function testProfile(name: string, extra: string[] = []): Promise<Profile> {
  return api<Profile>("POST", "/models/profiles", {
    name,
    engine_id: original.engine_id,
    model_file: original.model_file,
    load_args: [...original.load_args, ...extra],
    sampler: original.sampler,
    reasoning: original.reasoning,
  });
}
/** The expert tab with `p` in the editor. */
async function editIn(page: Page, open: (hash: string) => Promise<void>, p: Profile) {
  await open("/settings/expert");
  await page.locator('[data-spot="expert-profile"]').getByRole("combobox", { name: "Profil" }).selectOption(String(p.id));
  await expect(row(page, p.name)).toContainText("wird bearbeitet");
}

test("Seitenleiste: Modellname, Quant und Kontext; Klick öffnet den Tab Modell", async ({ page, open }) => {
  const e = await engine();
  expect(e.model.name, "the test model has a name in its GGUF").toBeTruthy();
  await open("/");
  const pill = modelStatus(page);
  await expect(pill).toContainText(e.model.name!);
  await expect(pill).toContainText("Modell bereit"); // for screen readers, in front of the name
  if (!isPhone(page)) {
    await expect(pill).toContainText(`${e.model.quant} · ${ctxShort(e.model.ctx!)}`);
    await expect(pill).toHaveAttribute("title", new RegExp(e.model.file!.replace(/[.]/g, "\\.")));
  }
  await pill.click();
  await expect(page).toHaveURL(/#\/settings\/model$/);
  await expect(page.getByRole("heading", { name: "Dein Modell", level: 2 })).toBeVisible();
});

test("Tab Modell: das aktive Modell steht oben, mit Zustand, Kontext, Profil und den Aktionen", async ({ page, open }) => {
  const e = await engine();
  await open("/settings/model");
  const tabs = page.getByRole("navigation", { name: "Bereiche der Einstellungen" });
  await expect(tabs.getByRole("link", { name: "Modell", exact: true })).toHaveAttribute("aria-current", "page");
  const hero = page.locator('[data-spot="model"]');
  await expect(hero.getByText(e.model.name!, { exact: true })).toBeVisible();
  await expect(hero.getByText(e.model.name!, { exact: true })).toBeInViewport(); // on top, without scrolling
  await expect(hero).toContainText("Läuft");
  await expect(hero).toContainText(`${ctxShort(e.model.ctx!)} Tokens`);
  await expect(hero).toContainText(e.model.quant!);
  await expect(hero).toContainText(original.name);
  await expect(hero).toContainText(/geschätzt|gemessen ✓/);
  await expect(hero.getByRole("button", { name: "Modell wechseln" })).toBeInViewport();
  // Nothing technical on this tab: no start options, sampler numbers, engines or folders.
  await expect(page.getByRole("slider")).toHaveCount(0);
  await expect(page.locator('[data-spot="engines"], [data-spot="model-folders"], [data-spot="profiles"]')).toHaveCount(0);
  await expect(page.getByRole("link", { name: /Alles genau einstellen/ })).toHaveAttribute("href", "#/settings/expert");
  await snap(page, "settings-model", [page.locator(".mm-file")]);
  await page.getByRole("link", { name: /Alles genau einstellen/ }).click();
  await expect(page).toHaveURL(/#\/settings\/expert$/);
  await expect(tabs.getByRole("link", { name: "Experte" })).toHaveAttribute("aria-current", "page");
  const active = row(page, original.name);
  await expect(active).toContainText("aktiv");
  await expect(active).toContainText(`Kontext ${ctxShort(ctxOf(original.load_args) ?? e.model.ctx!)}`);
  await expect(active.getByRole("button", { name: /löschen$/ })).toHaveCount(0); // the active one cannot go
});

test("Experte: neues Profil, duplizieren, umbenennen, löschen", async ({ page, open }) => {
  const name = `${PREFIX}Profil ${Date.now().toString(36)}`;
  await open("/settings/expert");
  await page.getByRole("button", { name: "Neues Profil" }).click();
  const form = page.getByRole("form", { name: "Neues Profil" });
  // The model comes from "Meine Modelle" (a choice, no path to type); the active one's file is preset.
  await expect(form.getByRole("combobox", { name: /^Modell/ })).toHaveValue(original.model_file);
  await form.getByRole("textbox", { name: "Name" }).fill(name);
  await form.getByRole("button", { name: "Profil anlegen" }).click();
  const mine = row(page, name);
  await expect(mine).toContainText("geschätzt");
  await expect(mine).not.toContainText("aktiv");
  // A new profile opens in the editor.
  await expect(mine).toContainText("wird bearbeitet");
  const made = (await profiles()).find((p) => p.name === name)!;
  await expect(page.locator('[data-spot="expert-profile"]').getByRole("combobox", { name: "Profil" })).toHaveValue(String(made.id));
  expect(made.load_args).toEqual(original.load_args);

  // Duplicate: a copy with the same settings, opened in the editor.
  await mine.getByRole("button", { name: `${name} duplizieren` }).click();
  const copy = row(page, `${name} (Kopie)`);
  await expect(copy).toContainText("wird bearbeitet");
  const dup = (await profiles()).find((p) => p.name === `${name} (Kopie)`)!;
  expect([dup.load_args, dup.sampler, dup.model_file]).toEqual([made.load_args, made.sampler, made.model_file]);

  // Rename, then delete with a confirmation.
  const renamed = `${name} neu`;
  await row(page, name).getByRole("button", { name: "Umbenennen" }).click();
  const rename = row(page, name).getByRole("form", { name: "Umbenennen" });
  await rename.getByRole("textbox", { name: "Neuer Name" }).fill(renamed);
  await rename.getByRole("button", { name: "Speichern" }).click();
  const again = row(page, renamed);
  await expect(again).toBeVisible();
  await again.getByRole("button", { name: `${renamed} löschen` }).click();
  await again.getByRole("button", { name: "Profil löschen" }).click();
  await expect(again).toHaveCount(0);
  expect((await profiles()).some((p) => p.name === renamed)).toBe(false);
});

test("Experte: Temperatur per Regler, Speichern, Standard; Verwerfen", async ({ page, open }) => {
  const p = await testProfile(`${PREFIX}Sampler ${Date.now().toString(36)}`);
  await editIn(page, open, p);
  const answer = page.locator('[data-spot="sampler"]');
  const field = answer.getByRole("spinbutton", { name: /^Temperatur/ });
  const slider = answer.getByRole("slider", { name: "Temperatur" });
  const before = p.sampler.temperature ?? 0.8;
  // Two steps of 0.05 with the arrow keys on the slider.
  await slider.focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  const wanted = Math.round((before + 0.1) * 100) / 100;
  await expect(field).toHaveValue(String(wanted));
  await expect(bar(page)).toContainText("Ungespeicherte Änderungen");
  expect((await profile(p.id)).sampler.temperature, "nothing is saved before „Speichern“").toBe(p.sampler.temperature);
  await bar(page).getByRole("button", { name: "Speichern" }).click();
  await expect(bar(page)).toContainText("Gespeichert");
  await expect.poll(async () => (await profile(p.id)).sampler.temperature).toBe(wanted);
  expect((await profile(p.id)).load_args, "a sampler change leaves the start options alone").toEqual(p.load_args);

  // "Standard" leaves the value out: the profile no longer has one, the field shows the default greyed.
  await answer.getByRole("button", { name: "Temperatur auf Standard" }).click();
  await expect(field).toHaveValue("");
  await expect(answer.locator('[data-spot="temperature"] .mx-std')).toHaveText("Standard");
  await bar(page).getByRole("button", { name: "Speichern" }).click();
  await expect.poll(async () => (await profile(p.id)).sampler.temperature ?? null).toBe(null);
  await page.reload();
  await page.locator('[data-spot="expert-profile"]').getByRole("combobox", { name: "Profil" }).selectOption(String(p.id));
  await expect(page.locator('[data-spot="sampler"]').getByRole("spinbutton", { name: /^Temperatur/ })).toHaveValue("");

  // "Verwerfen" takes a draft back; the profile stays as it is.
  const topK = page.locator('[data-spot="sampler"]').getByRole("spinbutton", { name: /^Top-K/ });
  const k = await topK.inputValue();
  await topK.fill("12");
  await expect(bar(page)).toContainText("Ungespeicherte Änderungen");
  await expect(page.locator('[data-spot="expert-profile"]').getByRole("combobox", { name: "Profil" }), "another profile waits for save or discard").toBeDisabled();
  await bar(page).getByRole("button", { name: "Verwerfen" }).click();
  await expect(topK).toHaveValue(k);
  await expect(page.locator(".mx-save"), "nothing left to save").not.toHaveClass(/dirty/);
  // On a phone a save bar with nothing to save steps aside; elsewhere it stays with "Speichern" off.
  if (!isPhone(page)) await expect(bar(page).getByRole("button", { name: "Speichern" })).toBeDisabled();
  expect((await profile(p.id)).sampler.top_k).toBe(p.sampler.top_k);
});

test("Experte: Startoption schreibt load_args, unbekannte Argumente bleiben stehen", async ({ page, open }) => {
  const extra = ["--no-warmup", "--keep", "64"];
  const p = await testProfile(`${PREFIX}Start ${Date.now().toString(36)}`, extra);
  await editIn(page, open, p);
  const start = page.locator('[data-spot="start-options"]');
  await expect(page.getByText(/Gilt, sobald dieses Profil aktiv ist/).first(), "an inactive profile does not restart anything").toBeVisible();
  // The arguments without a control of their own show as text, one option per line.
  const rest = start.getByRole("textbox", { name: /^Weitere Argumente/ });
  await expect(rest).toHaveValue(/--no-warmup\n--keep 64/);

  // The context as a number: the -c pair moves to the end, every other argument keeps its place.
  await start.getByRole("textbox", { name: /^Kontext/ }).fill("3072");
  await expect(bar(page)).toContainText("Ungespeichert: Kontext");
  await bar(page).getByRole("button", { name: "Speichern" }).click();
  const withoutCtx = p.load_args.filter((_, i, a) => !(a[i] === "-c" || a[i - 1] === "-c"));
  await expect.poll(async () => (await profile(p.id)).load_args).toEqual([...withoutCtx, "-c", "3072"]);

  // A further argument typed as text joins the others; the controls' options stay.
  await rest.fill(`${await rest.inputValue()}\n--cache-reuse 256`);
  await expect(bar(page)).toContainText("Weitere Argumente");
  await bar(page).getByRole("button", { name: "Speichern" }).click();
  await expect.poll(async () => (await profile(p.id)).load_args.slice(-2)).toEqual(["--cache-reuse", "256"]);
  const saved = (await profile(p.id)).load_args;
  expect(saved.join(" ")).toContain("--no-warmup --keep 64 --cache-reuse 256");
  expect(ctxOf(saved)).toBe(3072);

  // A flag the app sets itself is refused before it reaches the server.
  await rest.fill(`${await rest.inputValue()}\n--port 1234`);
  await expect(bar(page)).toContainText("--port setzt die App selbst");
  await expect(bar(page).getByRole("button", { name: "Speichern" })).toBeDisabled();
  await bar(page).getByRole("button", { name: "Verwerfen" }).click();
  await expect(rest).not.toHaveValue(/--port/);
  expect((await profile(p.id)).load_args).toEqual(saved);
  await page.locator('[data-spot="context"]').evaluate((el) => el.scrollIntoView({ block: "start", behavior: "instant" }));
  // This PC's free memory; the profile's name (with the run's time) in the save bar.
  await snap(page, "settings-expert-start", [page.locator(".ms-mem"), bar(page).getByRole("status")]);
});

test("Profil-Wechsel ohne Neustart: Fortschritt sichtbar, danach neuer Kontext", async ({ page, open }) => {
  test.skip(!(await engine()).managed, "the test server does not manage the model (start it with --manage-engine)");
  const ctx = ctxOf(original.load_args) ?? 4096;
  const other = ctx === 2048 ? 3072 : 2048;
  const args = [...original.load_args.filter((_, i, a) => !(a[i] === "-c" || a[i - 1] === "-c")), "-c", String(other)];
  const name = `${PREFIX}Kontext ${ctxShort(other)}`;
  await api("POST", "/models/profiles", { name, engine_id: original.engine_id, model_file: original.model_file, load_args: args, sampler: original.sampler, reasoning: original.reasoning });

  await open("/settings/expert");
  const target = row(page, name);
  await expect(target).toContainText(`Kontext ${ctxShort(other)}`);
  await target.getByRole("button", { name: "Aktivieren" }).click();
  // While the server restarts: the steps in the row, "Wechselt …" in the sidebar or top bar.
  await expect(target.getByRole("status", { name: "Wechsel" })).toContainText("Neues Modell laden");
  await expect(modelStatus(page)).toContainText("Wechselt …");
  await expect(target.getByRole("status", { name: "Wechsel" })).toContainText(`Läuft · Kontext ${ctxShort(other)}`, { timeout: 120_000 });
  await expect(target).toContainText("aktiv");
  await expect(modelStatus(page)).not.toContainText("Wechselt");
  if (!isPhone(page)) await expect(modelStatus(page)).toContainText(`· ${ctxShort(other)}`);
  await page.getByRole("navigation", { name: "Bereiche der Einstellungen" }).getByRole("link", { name: "Modell", exact: true }).click();
  await expect(page.locator('[data-spot="model"]')).toContainText(`${ctxShort(other)} Tokens`);
  await expect(page.locator('[data-spot="model"]')).toContainText(name);

  // And back, the same way.
  await open("/settings/expert");
  await row(page, original.name).getByRole("button", { name: "Aktivieren" }).click();
  await expect(row(page, original.name).getByRole("status", { name: "Wechsel" })).toContainText(`Läuft · Kontext ${ctxShort(ctx)}`, { timeout: 120_000 });
  await readyWith(ctx);
});

test("Strg K: Modell, Experte, Profil wechseln, Temperatur, Antwortlänge, Top-P", async ({ page, open }) => {
  await open("/chars");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  const find = async (query: string, label: string, tab: "model" | "expert") => {
    await page.keyboard.press("Control+k");
    await dialog.getByRole("combobox").fill(query);
    await expect(dialog.getByRole("option").first()).toContainText(label);
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect(page).toHaveURL(new RegExp(`#/settings/${tab}$`));
  };
  await find("Modell", "Modell", "model");
  await expect(page.locator('[data-spot="model"]')).toBeInViewport();

  await find("Experte", "Experte", "expert");
  await expect(page.locator('[data-spot="profiles"]')).toBeInViewport();

  await find("Profil wechseln", "Profil wechseln", "expert");
  await expect(page.locator('[data-spot="profiles"]')).toBeInViewport();

  // The place of one value: its field gets the focus.
  const answer = page.locator('[data-spot="sampler"]');
  await find("Temperatur", "Temperatur", "expert");
  await expect(answer.getByRole("spinbutton", { name: /^Temperatur/ })).toBeFocused();
  await expect(answer.getByRole("spinbutton", { name: /^Temperatur/ })).toBeInViewport();

  await find("Antwortlänge", "Antwortlänge", "expert");
  await expect(answer.getByRole("spinbutton", { name: /^Antwortlänge/ })).toBeFocused();

  await find("Top-P", "Top-P", "expert");
  await expect(answer.getByRole("spinbutton", { name: /^Top-P/ })).toBeFocused();
});
