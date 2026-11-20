// "Meine Modelle" in the settings tab "Modell" (behind "Modell wechseln"): the list of model files
// (tiny GGUF files written into the test server's own app folder, plus the folder of the test model),
// split models and projectors, "Benutzen", deleting with a confirmation. In the tab "Experte": the
// model folders, the context with its memory, applying start options with a restart (only when the
// test server manages the model: `--manage-engine`) and Ctrl+K. Never touches a real LM Studio folder:
// start the test server with RP_LMSTUDIO_DIR set to an empty folder (README).

import { existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { api } from "./api";
import { expect, snap, test } from "./fixtures";
import { writeMiniGguf } from "./gguf";
import type { Locator, Page } from "@playwright/test";

type Profile = { id: number; name: string; model_file: string; load_args: string[]; sampler: Record<string, number>; reasoning: string; active: boolean };
type Folder = { path: string; exists: boolean };
type Local = {
  folders: { app: Folder & { default: boolean }; lmstudio: Folder | null; extra: Folder[] };
  models: { path: string; name: string | null; origin: string; profiles: { id: number; active: boolean }[] }[];
};

const PREFIX = "E2E ";
const DATA = process.env.RP_E2E_DATA ?? "";
const profiles = async () => (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles;
const engine = () => api<{ status: string; managed: boolean; model: { ctx: number | null } }>("GET", "/engine");
const local = () => api<Local>("GET", "/models/local");

let original: Profile;
let folders: Local["folders"];
/** The test server's app folder (default `<data>\models`) and the files written into it. */
let app = "";
const mini = () => join(app, "e2e", "E2E-Mini-Q4_K_M.gguf");
const split = (n: number) => join(app, "e2e", "teile", `E2E-Teile-Q4_K_M-0000${n}-of-00002.gguf`);

async function readyWith(ctx: number) {
  await expect
    .poll(async () => {
      const e = await engine();
      return e.status === "ready" ? e.model.ctx : null;
    }, { timeout: 120_000, intervals: [500, 1000, 2000] })
    .toBe(ctx);
}

function ctxOf(args: string[]): number | null {
  const i = args.findIndex((a) => a === "-c" || a === "--ctx-size");
  return i >= 0 ? Number(args[i + 1]) : null;
}

test.beforeAll(async () => {
  test.skip(!DATA, "RP_E2E_DATA is not set (see README)");
  const p = (await profiles()).find((x) => x.active);
  if (!p) throw new Error("no active model profile on the test server");
  original = p;
  folders = (await local()).folders;
  app = folders.app.path;
});

test.beforeEach(async () => {
  writeMiniGguf(mini(), { name: "E2E Mini", ctx: 32768, layers: 8, bytes: 2_500_000 });
  writeMiniGguf(join(app, "e2e", "mmproj-E2E-Mini-F16.gguf"), { name: "E2E Projektor", arch: "clip", bytes: 1000 });
  writeMiniGguf(split(1), { name: "E2E Teile", arch: "qwen3moe", ctx: 40960, layers: 8, bytes: 1_000_000 });
  writeMiniGguf(split(2), { name: "", arch: "qwen3moe", bytes: 1_000_000 });
  // The folder of the test model, so it is in the list with its profile.
  await api("PUT", "/models/folders", { app_folder: null, extra: [dirname(original.model_file)] });
});

// Back to the profile and settings the suite started with; no test profile or file left behind.
test.afterEach(async () => {
  const now = (await profiles()).find((x) => x.active);
  const ctx = ctxOf(original.load_args);
  const managed = (await engine()).managed;
  if (now?.id !== original.id) {
    await api("POST", `/models/profiles/${original.id}/activate`);
    if (managed && ctx) await readyWith(ctx);
  }
  const back = (await profiles()).find((x) => x.id === original.id)!;
  if (JSON.stringify(back.load_args) !== JSON.stringify(original.load_args) || JSON.stringify(back.sampler) !== JSON.stringify(original.sampler)) {
    await api("PUT", `/models/profiles/${original.id}`, { load_args: original.load_args, sampler: original.sampler, reasoning: original.reasoning });
    if (managed && ctx) await readyWith(ctx);
  }
  for (const p of await profiles()) if (p.name.startsWith(PREFIX)) await api("DELETE", `/models/profiles/${p.id}`);
  await api("PUT", "/models/folders", { app_folder: folders.app.default ? null : folders.app.path, extra: folders.extra.map((f) => f.path) });
  rmSync(join(app, "e2e"), { recursive: true, force: true });
  rmSync(join(DATA, "e2e-extra"), { recursive: true, force: true });
  rmSync(join(DATA, "e2e-app"), { recursive: true, force: true });
});

const list = (page: Page) => page.getByRole("list", { name: "Meine Modelle" });
const model = (page: Page, name: string): Locator => list(page).getByRole("listitem", { name: `Modell ${name}` });
const start = (page: Page) => page.locator('[data-spot="start-options"]');
const bar = (page: Page) => page.getByRole("group", { name: "Speichern" });
/** The tab "Modell" with "Meine Modelle" open. */
async function openList(page: Page, open: (hash: string) => Promise<void>) {
  await open("/settings/model");
  await page.getByRole("button", { name: "Modell wechseln" }).click();
  await expect(page.getByRole("heading", { name: "Meine Modelle", level: 2 })).toBeVisible();
}
/** Paths and the free VRAM differ between machines and runs. */
const masks = (page: Page) => [page.locator(".mf-path, .ms-mem, .lm-where, .mm-file")];

test("Meine Modelle: Name, Quant, Größe, Ordner; Mehrteiler als ein Modell, Projektor als Zusatz", async ({ page, open }) => {
  await openList(page, open);
  const m = model(page, "E2E Mini");
  await expect(m).toContainText("Q4_K_M");
  await expect(m).toContainText("2 MB");
  await expect(m).toContainText("App-Ordner");
  await expect(m).toContainText("32k Kontext");
  await expect(m).toContainText("+ Bild-Projektor");
  await expect(m.getByRole("button", { name: "Benutzen" })).toBeEnabled();
  await expect(m.getByRole("button", { name: "E2E Mini löschen" })).toBeVisible();
  await expect(model(page, "E2E Teile")).toContainText("2 Teile");
  await expect(list(page).getByRole("listitem", { name: /Projektor/ })).toHaveCount(0);
  // The test model: in the added folder, used by the active profile, so neither "Benutzen" nor "Löschen".
  const name = (await local()).models.find((x) => x.profiles.some((p) => p.active))!.name!;
  const running = model(page, name);
  await expect(running).toContainText("aktiv");
  await expect(running).toContainText("Weiterer Ordner");
  await expect(running.getByRole("button", { name: "Benutzen" })).toHaveCount(0);
  await expect(running.getByRole("button", { name: /löschen$/ })).toHaveCount(0);
  await expect(running.getByRole("button", { name: /^Ordner von .* öffnen$/ })).toBeVisible();
  await page.locator('[data-spot="local-models"]').scrollIntoViewIfNeeded();
  await snap(page, "settings-models-local", masks(page));
});

test("Benutzen: ein Modell ohne Profil bekommt eines (Startoptionen vom aktiven) und wird aktiv", async ({ page, open }) => {
  await openList(page, open);
  const before = (await profiles()).length;
  await model(page, "E2E Mini").getByRole("button", { name: "Benutzen" }).click();
  await expect.poll(async () => (await profiles()).find((p) => p.name === "E2E Mini")?.active ?? false).toBe(true);
  const made = (await profiles()).find((p) => p.name === "E2E Mini")!;
  expect((await profiles()).length).toBe(before + 1);
  expect(made.model_file.toLowerCase()).toBe(mini().toLowerCase());
  // CPU stays CPU: the GPU layers come from the active profile; the context fits the new model.
  const ngl = original.load_args.indexOf("-ngl");
  if (ngl >= 0) expect(made.load_args.slice(made.load_args.indexOf("-ngl"), made.load_args.indexOf("-ngl") + 2)).toEqual(original.load_args.slice(ngl, ngl + 2));
  expect(ctxOf(made.load_args)).toBe(8192);
  await expect(model(page, "E2E Mini")).toContainText("aktiv");
  await expect(page.locator('[data-spot="model"]')).toContainText("E2E Mini"); // the card names the new profile
  // Back to the test model by its row: its existing profile is used again, no new one.
  await page.reload();
  await page.getByRole("button", { name: "Modell wechseln" }).click();
  const name = (await local()).models.find((x) => x.profiles.some((p) => p.id === original.id))!.name!;
  await model(page, name).getByRole("button", { name: "Benutzen" }).click();
  await expect.poll(async () => (await profiles()).find((p) => p.active)?.id).toBe(original.id);
  expect((await profiles()).length).toBe(before + 1);
});

test("Löschen: nur im App-Ordner, mit Rückfrage und Größe, alle Teile", async ({ page, open }) => {
  await openList(page, open);
  const m = model(page, "E2E Teile");
  await m.getByRole("button", { name: "E2E Teile löschen" }).click();
  const confirm = m.getByRole("group", { name: "Löschen bestätigen" });
  await expect(confirm).toContainText("MB werden frei (alle 2 Teile)");
  await confirm.getByRole("button", { name: "Abbrechen" }).click();
  await expect(confirm).toHaveCount(0);
  expect(existsSync(split(1))).toBe(true);
  await m.getByRole("button", { name: "E2E Teile löschen" }).click();
  await m.getByRole("button", { name: "Modell löschen" }).click();
  await expect(model(page, "E2E Teile")).toHaveCount(0);
  expect(existsSync(split(1)) || existsSync(split(2))).toBe(false);
  // Outside the app folder the server refuses, whatever the page asks.
  const res = await fetch(`${process.env.RP_E2E_URL ?? "http://127.0.0.1:7861"}/api/models/local`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${process.env.RP_E2E_TOKEN ?? ""}`, "Content-Type": "application/json" },
    body: JSON.stringify({ path: original.model_file }),
  });
  expect(res.status).toBe(403);
  expect(existsSync(original.model_file)).toBe(true);
});

test("Modellordner: weiteren Ordner hinzufügen und entfernen, App-Ordner ändern und zurück", async ({ page, open }) => {
  const extra = join(DATA, "e2e-extra");
  writeMiniGguf(join(extra, "E2E-Extra-Q8_0.gguf"), { name: "E2E Extra", bytes: 2000 });
  await open("/settings/expert");
  const panel = page.locator('[data-spot="model-folders"]');
  const listed = async (name: string) => (await local()).models.find((x) => x.name === name)?.origin ?? null;
  await panel.getByRole("button", { name: "Ordner hinzufügen" }).click();
  const form = panel.getByRole("form", { name: "Ordner hinzufügen" });
  await form.getByRole("textbox", { name: /^Ordner mit Modellen/ }).fill(extra);
  await form.getByRole("button", { name: "Hinzufügen" }).click();
  await expect(panel.getByText(extra, { exact: true })).toBeVisible();
  expect(await listed("E2E Extra")).toBe("extra");
  // "Meine Modelle" on the tab "Modell" lists it, without "Löschen" (not the app folder).
  await openList(page, open);
  await expect(model(page, "E2E Extra")).toContainText("Weiterer Ordner");
  await expect(model(page, "E2E Extra").getByRole("button", { name: /löschen$/ })).toHaveCount(0);
  await open("/settings/expert");
  await panel.getByRole("button", { name: `Ordner ${extra} entfernen` }).click();
  await expect(panel.getByText(extra, { exact: true })).toHaveCount(0);
  expect(await listed("E2E Extra")).toBe(null);

  // A folder that does not exist is refused with a message.
  await panel.getByRole("button", { name: "Ordner hinzufügen" }).click();
  await form.getByRole("textbox", { name: /^Ordner mit Modellen/ }).fill(join(DATA, "gibt-es-nicht"));
  await form.getByRole("button", { name: "Hinzufügen" }).click();
  await expect(form).toContainText("Diesen Ordner gibt es nicht");
  await form.getByRole("button", { name: "Abbrechen" }).click();

  // The app folder moves (created when missing) and back to the default.
  const moved = join(DATA, "e2e-app");
  await panel.getByRole("button", { name: "Ändern" }).click();
  const change = panel.getByRole("form", { name: "App-Ordner ändern" });
  await change.getByRole("textbox", { name: /^Neuer App-Ordner/ }).fill(moved);
  await change.getByRole("button", { name: "Speichern" }).click();
  await expect(panel.getByText(moved, { exact: true })).toBeVisible();
  expect(await listed("E2E Mini")).toBe(null); // it lies in the old app folder
  expect(existsSync(moved)).toBe(true);
  await panel.getByRole("button", { name: "Standard" }).click();
  await expect(panel.getByText(moved, { exact: true })).toHaveCount(0);
  expect(await listed("E2E Mini")).toBe("app");
});

test("Experte: Kontext mit Speicherbedarf; Regler, Feld und Verwerfen", async ({ page, open }) => {
  await open("/settings/expert");
  const s = start(page);
  await expect(s.getByText(/^ca\. [\d,]+ GB VRAM/)).toBeVisible();
  // The slider runs over the steps with an estimate; the field shows the number it sets.
  const slider = s.getByRole("slider", { name: "Kontext" });
  await slider.focus();
  await page.keyboard.press("Home"); // the smallest step: 2k
  await expect(s.getByRole("textbox", { name: /^Kontext/ })).toHaveValue("2048");
  await expect(slider).toHaveAttribute("aria-valuetext", "2k Tokens");
  await expect(s.getByText(/^ca\. [\d,]+ GB VRAM/)).toBeVisible();
  await expect(bar(page)).toContainText("Ungespeichert: Kontext");
  await expect(bar(page)).toContainText("Speichern startet das Modell neu");
  await expect(s.locator('[data-spot="context"] .ms-restart')).toHaveText("Neustart");
  await s.locator('[data-spot="context"]').evaluate((el) => el.scrollIntoView({ block: "start", behavior: "instant" }));
  await snap(page, "settings-expert-context", masks(page));
  await bar(page).getByRole("button", { name: "Verwerfen" }).click();
  await expect(s.getByRole("textbox", { name: /^Kontext/ })).toHaveValue(String(ctxOf(original.load_args) ?? ""));
  await expect(page.locator(".mx-save"), "nothing left to save").not.toHaveClass(/dirty/);
  expect((await profiles()).find((p) => p.id === original.id)!.load_args).toEqual(original.load_args);
});

test("Experte: Startoptionen speichern startet das Modell neu, der neue Kontext kommt an", async ({ page, open }) => {
  test.skip(!(await engine()).managed, "the test server does not manage the model (start it with --manage-engine)");
  test.setTimeout(180_000); // two restarts of the model: to 2k here, back in afterEach
  const ctx = ctxOf(original.load_args) ?? 4096;
  await open("/settings/expert");
  const s = start(page);
  // Never more on the GPU than the test profile has (CPU only).
  const ngl = original.load_args.indexOf("-ngl");
  await expect(s.getByRole("textbox", { name: /^GPU-Schichten/ })).toHaveValue(ngl >= 0 ? original.load_args[ngl + 1] : "");
  await s.getByRole("combobox", { name: /^KV-Cache K/ }).selectOption("q8_0");
  await s.getByRole("combobox", { name: /^KV-Cache V/ }).selectOption("q8_0");
  const slider = s.getByRole("slider", { name: "Kontext" });
  await slider.focus();
  await page.keyboard.press("Home"); // the smallest step: 2k
  await expect(slider).toHaveAttribute("aria-valuetext", "2k Tokens");
  await expect(bar(page)).toContainText("Kontext, KV-Cache");
  await bar(page).getByRole("button", { name: "Speichern" }).click();
  await expect(s.getByRole("status", { name: "Wechsel" })).toContainText(/Neues Modell laden|Läuft/);
  // What the model server itself says (`/props` n_ctx through /engine): the new context.
  await readyWith(2048);
  await expect(s.getByRole("status", { name: "Wechsel" })).toContainText("Läuft · Kontext 2k", { timeout: 30_000 });
  const saved = (await profiles()).find((p) => p.id === original.id)!;
  expect(saved.load_args).toEqual(expect.arrayContaining(["-ctk", "q8_0", "-ctv", "q8_0", "-c", "2048"]));
  expect(ctxOf(saved.load_args)).toBe(2048);
  await open("/settings/model");
  await expect(page.locator('[data-spot="model"]')).toContainText("2k Tokens");
  expect(ctx).toBeGreaterThan(2048);
});

test("Strg K: Kontext, Kreativität, GPU-Schichten, KV-Cache, Modellordner, Meine Modelle", async ({ page, open }) => {
  await open("/chars");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  const find = async (query: string, spot: string, tab: "model" | "expert") => {
    await page.keyboard.press("Control+k");
    await dialog.getByRole("combobox").fill(query);
    await expect(dialog.getByRole("option").first()).toContainText(query);
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect(page).toHaveURL(new RegExp(`#/settings/${tab}$`));
    await expect(page.locator(`[data-spot="${spot}"]`)).toBeInViewport();
  };
  await find("Kontext", "context", "expert");
  await expect(start(page).getByRole("textbox", { name: /^Kontext/ })).toBeFocused();
  await find("Kreativität", "temperature", "expert");
  await expect(page.locator('[data-spot="sampler"]').getByRole("spinbutton", { name: /^Temperatur/ })).toBeFocused();
  await find("GPU-Schichten", "gpu-layers", "expert");
  await expect(start(page).getByRole("textbox", { name: /^GPU-Schichten/ })).toBeFocused();
  await find("KV-Cache", "kv-cache", "expert");
  await find("Modellordner", "model-folders", "expert");
  await expect(page.getByRole("button", { name: "Ordner hinzufügen" })).toBeFocused();
  // "Meine Modelle" opens the list behind "Modell wechseln" on the tab "Modell".
  await find("Meine Modelle", "local-models", "model");
  await expect(page.getByRole("heading", { name: "Meine Modelle", level: 2 })).toBeVisible();
});
