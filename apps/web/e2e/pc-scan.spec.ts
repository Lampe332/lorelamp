// "Dein PC" and "Engines" in the settings tab "Experte": the scan shows GPU, memory, CPU and the
// suggested build; the engine list matches the server; one real download of the newest official CPU
// build (desktop-graphit only: ~25 MB from GitHub) is installed, self-tested and removed again; Ctrl+K
// finds the places. The scan reads this PC; the expected values come from the server's own answer.

import { api } from "./api";
import { expect, test } from "./fixtures";

type Scan = {
  gpus: { name: string; vram_total: number; driver: string | null; driver_issue: string | null }[];
  ram_total: number;
  cpu: { cores: number; threads: number };
  backend: { suggested: string; cuda: string | null; gpu_index: number | null; reason: string };
};
type Engine = { id: number; name: string; origin: string; backend: string; exe: string; selftest: { ok: boolean } | null };
type Asset = { name: string; backend: string; variant: string };
type Release = { tag: string; assets: Asset[] };

const BACKEND: Record<string, string> = { cpu: "CPU", rocm: "ROCm", vulkan: "Vulkan" };
const gb = (bytes: number) => `${(Math.round((bytes / 1024 ** 3) * 10) / 10).toLocaleString("de-DE")} GB`;
const engines = async () => (await api<{ engines: Engine[] }>("GET", "/models/engines")).engines;

test("PC prüfen: Grafikkarte, Grafikspeicher, Arbeitsspeicher, Prozessor und Empfehlung", async ({ page, open }) => {
  await open("/settings/expert");
  const card = page.locator('[data-spot="pc"]');
  await card.getByRole("button", { name: "PC prüfen" }).click();
  await expect(card.getByRole("group", { name: "Empfehlung" })).toBeVisible({ timeout: 30_000 });
  await expect(card.getByRole("button", { name: "PC prüfen" })).toBeEnabled({ timeout: 30_000 });

  // What the page shows is what the server measured (and kept).
  const scan = (await api<{ scan: Scan }>("GET", "/system/scan")).scan;
  const main = scan.gpus[scan.backend.gpu_index ?? 0];
  await expect(card.locator(".mm-name")).toHaveText(main.name);
  const gpu = card.getByRole("group", { name: `Grafikkarte ${main.name}` });
  await expect(gpu).toContainText(`${gb(main.vram_total)} Grafikspeicher`);
  await expect(gpu).toContainText(/von anderen Programmen belegt|Belegung unbekannt/);
  if (main.driver) await expect(gpu).toContainText(`Treiber ${main.driver}`);
  // A known problem driver is said in words, with its version.
  if (main.driver_issue) await expect(gpu.locator('[data-spot="driver"]')).toContainText(`Grafiktreiber ${main.driver}`);
  else await expect(gpu.locator('[data-spot="driver"]')).toHaveCount(0);
  await expect(card).toContainText(gb(scan.ram_total));
  await expect(card).toContainText(`${scan.cpu.cores} Kerne`);
  await expect(card).toContainText(`${scan.cpu.threads} Threads`);
  const label = scan.backend.suggested === "cuda" ? `CUDA ${scan.backend.cuda}` : BACKEND[scan.backend.suggested];
  const pick = card.getByRole("group", { name: "Empfehlung" });
  await expect(pick).toContainText(`Empfohlen: ${label}`);
  await expect(pick).toContainText(scan.backend.reason);

  // The stored scan is shown again after a reload, without scanning.
  await page.reload();
  await expect(page.locator('[data-spot="pc"] .mm-name')).toHaveText(main.name);
});

test("Engines: Liste wie auf dem Server, mit Herkunft und Selbsttest", async ({ page, open }) => {
  await open("/settings/expert");
  const list = page.getByRole("list", { name: "Engines" });
  const all = await engines();
  await expect(list.getByRole("listitem")).toHaveCount(all.length);
  for (const e of all) {
    const item = list.getByRole("listitem", { name: `Engine ${e.name}` }).first();
    await expect(item).toBeVisible();
    if (e.origin === "own") {
      await expect(item).toContainText("kein Selbsttest der App");
      await expect(item.getByRole("button", { name: /entfernen$/ })).toHaveCount(0);
    } else if (e.selftest?.ok) {
      await expect(item).toContainText(/Selbsttest bestanden|Dateien geprüft/);
    }
  }
});

test("Engine herunterladen: offizieller CPU-Build, Selbsttest, wieder entfernen", async ({ page, open }, info) => {
  test.skip(info.project.name !== "desktop-graphit", "one real download per run is enough");
  test.setTimeout(180_000);
  const releases = (await api<{ releases: Release[] }>("GET", "/system/engine-releases")).releases;
  const newest = releases[0];
  const cpu = newest.assets.find((a) => a.backend === "cpu");
  expect(cpu, "the newest release has a Windows CPU zip").toBeTruthy();
  // A build left over from an earlier run goes first, so this run really downloads.
  for (const e of await engines()) if (e.origin === "official" && e.exe.includes(`${newest.tag}-${cpu!.variant}`)) await api("DELETE", `/models/engines/${e.id}`);

  await open("/settings/expert");
  const panel = page.locator('[data-spot="engines"]');
  await panel.getByRole("button", { name: "Engine herunterladen" }).click();
  const dl = panel.getByRole("region", { name: "Engine herunterladen" });
  await dl.getByRole("button", { name: newest.tag, exact: true }).click();
  await dl.locator("label.choice").filter({ hasText: "CPU" }).first().click();
  await expect(dl.getByRole("radio", { name: /^CPU/ })).toBeChecked();
  await dl.getByRole("button", { name: /^Herunterladen/ }).click();

  await expect(dl.getByRole("status")).toContainText(`Installiert: llama.cpp ${newest.tag} (CPU). Selbsttest bestanden`, { timeout: 150_000 });
  const added = (await engines()).find((e) => e.origin === "official" && e.exe.includes(`${newest.tag}-${cpu!.variant}`));
  expect(added?.selftest?.ok).toBe(true);
  const item = page.getByRole("list", { name: "Engines" }).getByRole("listitem", { name: `Engine ${added!.name}` }).last();
  await expect(item).toContainText("Selbsttest bestanden");
  // The same build cannot be fetched twice.
  await expect(dl.getByRole("radio", { name: /^CPU/ })).toBeDisabled();

  await item.getByRole("button", { name: `${added!.name} entfernen` }).click();
  await item.getByRole("button", { name: "Engine entfernen" }).click();
  await expect.poll(async () => (await engines()).some((e) => e.id === added!.id)).toBe(false);
  await expect(page.getByRole("list", { name: "Engines" }).getByRole("listitem")).toHaveCount((await engines()).length);
});

test("Strg K: PC prüfen, Engine, Grafikkarte, Treiber", async ({ page, open }) => {
  await open("/chars");
  const dialog = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  const find = async (query: string, label: string, spot: string) => {
    await page.keyboard.press("Control+k");
    await dialog.getByRole("combobox").fill(query);
    await expect(dialog.getByRole("option").first()).toContainText(label);
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect(page).toHaveURL(/#\/settings\/expert$/);
    await expect(page.locator(`[data-spot="${spot}"]`)).toBeInViewport();
  };
  await find("PC prüfen", "PC prüfen", "pc");
  await expect(page.locator('[data-spot="pc"]').getByRole("button", { name: "PC prüfen" })).toBeFocused();
  await find("Engine", "Engine", "engines");
  await expect(page.getByRole("button", { name: "Engine herunterladen" })).toBeFocused();
  await find("Grafikkarte", "Grafikkarte", "pc");
  await find("Treiber", "Treiber", "pc");
});
