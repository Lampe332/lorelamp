// R2: why the model does not run, said where the user looks, and the benchmark's honest cards.
// - Another program holds the model server's port: the tab "Modell" and the model status say so,
//   "Port ändern" leads to the port field (tab "Experte"), the other program is left alone, and back
//   on a free port the model runs again.
// - The port field takes 1024–65535 only (here and at the server), and Ctrl+K finds it.
// - A model that crashes at every start: the supervisor stops for good (no start after the restart
//   pause), the card says so with the number of crashes, and "Neu starten" asks for a new series.
// - The benchmark's cards: the text comes from the row each card shows (no "ab 8k" for a 4k row),
//   recommendations of the same row share one card, a CPU run names the RAM only, and steps the time
//   did not reach are named.
// Needs a test server that manages the model (`--manage-engine`) with the shortened benchmark runs
// (see benchmark.spec.ts) and RP_E2E_DATA. All server answers are real; the stored benchmark results
// of the last test are written into the test database in the form the server stores them.

import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { createConnection, createServer, type Server, type Socket } from "node:net";
import { DatabaseSync } from "node:sqlite";
import { dirname, join } from "node:path";
import type { Page } from "@playwright/test";
import { api } from "./api";
import { dataDir } from "./db";
import { expect, isPhone, modelStatus, test } from "./fixtures";
import { miniGguf } from "./gguf";

type Problem = { code: "port_busy"; port: number } | { code: "gave_up"; crashes: number } | null;
type Engine = { status: string; managed: boolean; unloaded: boolean; port: number; problem: Problem; pid: number | null; bench_profile_id?: number | null; profile: { id: number } | null };
type Profile = { id: number; name: string; load_args: string[]; measured: boolean; active: boolean; model_file: string };
type Row = { ctx: number; kv: string; n_cpu_moe: number; pp: number | null; tg: number | null; overflow_mib: number | null; outcome: string; load_s: number | null };
type Result = {
  profile_id: number;
  at: number;
  duration_s: number;
  rows: Row[];
  picks: { purpose: string; row: number }[];
  moe: boolean;
  gpu: string | null;
  conditions?: { ram_available_mib: number | null; vram_free_mib: number | null; commit_free_mib: number | null } | null;
  unmeasured_stages?: number[];
  prompt_tokens?: number;
  gen_tokens?: number;
};
type Bench = { running: unknown; result: Result | null; results: Result[] };

const engine = () => api<Engine>("GET", "/engine");
const profiles = async () => (await api<{ profiles: Profile[] }>("GET", "/models/profiles")).profiles;
const active = async () => (await profiles()).find((p) => p.active)!;
const bench = (id: number) => api<Bench>("GET", `/models/profiles/${id}/benchmark`);
const SHOTS = process.env.RP_E2E_SHOTS;
const ctxShort = (n: number) => `${new Intl.NumberFormat("de-DE").format(Math.round(n / 1024))}k`;
const BROKEN = "R2 Defekt";

let startPort: number | null = null;
let original: Profile | null = null;

async function shot(page: Page, name: string, theme: string) {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: join(SHOTS, `r2-${name}-${isPhone(page) ? "375" : "1920"}-${theme}.png`) });
}

/** The model answers (on the start port), no benchmark runs. */
async function settled(timeout = 180_000) {
  await expect
    .poll(
      async () => {
        const e = await engine();
        return e.status === "ready" && !e.bench_profile_id && !e.problem && (startPort === null || e.port === startPort);
      },
      { timeout, intervals: [500, 1000, 2000] },
    )
    .toBe(true);
}

test.beforeEach(async () => {
  const e = await engine();
  test.skip(!e.managed, "the test server does not manage the model (start it with --manage-engine)");
  startPort ??= e.port;
  original ??= await active();
});

// The port, the profile and the model as before; no test profile or file left behind.
test.afterEach(async () => {
  if (startPort === null || !original) return;
  if ((await engine()).port !== startPort) await api("PUT", "/settings", { engine_port: startPort });
  if ((await active()).id !== original.id) await api("POST", `/models/profiles/${original.id}/activate`);
  for (const p of await profiles()) if (p.name === BROKEN) await api("DELETE", `/models/profiles/${p.id}`);
  await settled();
});

/** A program that holds a port and never answers like a model server. */
async function holdPort(): Promise<{ server: Server; port: number; close: () => Promise<void> }> {
  const open = new Set<Socket>();
  const server = createServer((s) => {
    open.add(s);
    s.on("close", () => open.delete(s));
    s.end();
  });
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  const addr = server.address();
  if (!addr || typeof addr === "string") throw new Error("no port");
  const close = () =>
    new Promise<void>((ok) => {
      for (const s of open) s.destroy();
      server.close(() => ok());
    });
  return { server, port: addr.port, close };
}

/** Something still accepts connections on `port`. */
const listening = (port: number) =>
  new Promise<boolean>((ok) => {
    const s = createConnection({ port, host: "127.0.0.1" }, () => {
      s.destroy();
      ok(true);
    });
    s.on("error", () => ok(false));
  });

test("Belegter Port: Hinweis im Tab Modell und am Modell-Status, der Lauscher bleibt, ein freier Port startet das Modell wieder", async ({ page, open, theme }) => {
  test.setTimeout(240_000);
  await settled();
  const { server, port: busy, close } = await holdPort();
  try {
    await api("PUT", "/settings", { engine_port: busy });
    await expect.poll(async () => (await engine()).problem, { timeout: 60_000 }).toEqual({ code: "port_busy", port: busy });

    await open("/settings/model");
    const hero = page.locator("section.mx-hero");
    const note = hero.locator(".r2-problem");
    await expect(note).toContainText(`Der Port ${busy} wird von einem anderen Programm benutzt.`);
    await expect(note).toContainText("Die App lässt das andere Programm in Ruhe");
    await expect(hero.locator(".mm-state")).toHaveText("Port belegt");
    await expect(hero.locator(".unload-state")).toHaveText("Nicht geladen");
    if (isPhone(page)) await expect(modelStatus(page)).toContainText("Port belegt");
    else await expect(modelStatus(page)).toContainText(`Der Port ${busy} wird von einem anderen Programm benutzt`);
    await shot(page, "port-busy", theme);

    // Nothing was started, and the other program still holds its port.
    expect((await engine()).pid).toBeNull();
    expect(server.listening).toBe(true);
    expect(await listening(busy)).toBe(true);

    // "Port ändern" leads to the field in the tab "Experte", which shows the held port.
    await note.getByRole("link", { name: "Port ändern" }).click();
    await expect(page).toHaveURL(/#\/settings\/expert/);
    const field = page.getByLabel("Port des Modell-Servers");
    await expect(field).toBeInViewport();
    await expect(field).toHaveValue(String(busy));
    const form = page.locator(".r2-port");
    await expect(form).toContainText(`Port ${busy} benutzt gerade ein anderes Programm.`);
    await shot(page, "port-field-busy", theme);

    // Back to the free port: saved, the model starts there, the hint is gone.
    await field.fill(String(startPort));
    await form.getByRole("button", { name: "Übernehmen" }).click();
    await expect(form).toContainText(`Gespeichert. Das Modell startet jetzt auf Port ${startPort}.`);
    await settled(120_000);
    expect((await engine()).port).toBe(startPort);
    await open("/settings/model");
    await expect(page.locator("section.mx-hero .mm-state")).toHaveText("Läuft", { timeout: 30_000 });
    await expect(page.locator(".r2-problem")).toHaveCount(0);
    expect(await listening(busy)).toBe(true);
  } finally {
    await close();
  }
});

test("Port-Feld: 80 und 70000 werden abgelehnt (hier und am Server), Strg K findet es", async ({ page, open }) => {
  await open("/");
  if (isPhone(page)) await page.locator("header.top").getByRole("button", { name: "Suchen und Schnellaktionen" }).click();
  else await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Suchen und Schnellaktionen" });
  await expect(palette).toBeVisible();
  await page.keyboard.type("modell port");
  await palette.getByRole("option").filter({ hasText: "Modell-Port" }).first().click();
  await expect(page).toHaveURL(/#\/settings\/expert/);
  const field = page.getByLabel("Port des Modell-Servers");
  await expect(field).toBeInViewport();
  await expect(field).toHaveValue(String(startPort));
  const form = page.locator(".r2-port");
  for (const bad of ["80", "70000"]) {
    await field.fill(bad);
    await form.getByRole("button", { name: "Übernehmen" }).click();
    await expect(form.locator(".help")).toHaveText("Bitte einen Port von 1024 bis 65535.");
    await expect(field).toHaveAttribute("aria-invalid", "true");
    expect((await engine()).port).toBe(startPort);
    // The server refuses it as well.
    await expect(api("PUT", "/settings", { engine_port: Number(bad) })).rejects.toThrow(/400.*bad_port/);
  }
  expect((await engine()).port).toBe(startPort);
});

test("Wächter gibt auf: kein weiterer Start, ehrlicher Hinweis mit Zahl der Abstürze, „Neu starten“ versucht es wieder", async ({ page, open, theme }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA");
  test.setTimeout(300_000);
  await settled();
  // A model file no model server can load: a valid header with filler instead of weights.
  const folders = (await api<{ folders: { app: { path: string } } }>("GET", "/models/local")).folders;
  const file = join(folders.app.path, "e2e-r2", "r2-defekt.gguf");
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, miniGguf({ name: BROKEN, bytes: 4096 }));
  try {
    await api("POST", "/models/local/use", { path: file });
    await expect.poll(async () => (await engine()).problem?.code, { timeout: 150_000, intervals: [1000] }).toBe("gave_up");
    const gaveUp = (await engine()).problem as { code: "gave_up"; crashes: number };
    expect(gaveUp.crashes).toBeGreaterThanOrEqual(3);

    await open("/settings/model");
    const hero = page.locator("section.mx-hero");
    const note = hero.locator(".r2-problem");
    await expect(note).toContainText(`Das Modell ist ${gaveUp.crashes}-mal kurz hintereinander abgestürzt.`);
    await expect(note).toContainText("Die App startet es nicht mehr von selbst");
    await expect(note).not.toContainText("Kein weiterer Startversuch. Näheres"); // the server's own hint is not repeated
    await expect(hero.locator(".mm-state")).toHaveText("Modell angehalten");
    if (!isPhone(page)) await expect(modelStatus(page)).toContainText(`Nach ${gaveUp.crashes} Abstürzen kein weiterer Startversuch`);
    await shot(page, "gave-up", theme);

    // Honest: past the restart pause nothing starts by itself.
    if (theme === "graphit" && !isPhone(page)) {
      await page.waitForTimeout(15_000);
      const later = await engine();
      expect([later.problem?.code, later.pid]).toEqual(["gave_up", null]);
    }

    // "Neu starten": a new series begins (the supervisor no longer reports giving up).
    await note.getByRole("button", { name: "Neu starten" }).click();
    await expect.poll(async () => (await engine()).problem?.code ?? null, { timeout: 15_000, intervals: [200] }).toBeNull();
  } finally {
    if (original) await api("POST", `/models/profiles/${original.id}/activate`);
    await settled();
    for (const p of await profiles()) if (p.name === BROKEN) await api("DELETE", `/models/profiles/${p.id}`);
    rmSync(dirname(file), { recursive: true, force: true });
  }
});

/** The profile's stored benchmark results (`bench_result_<id>`, `bench_results_<id>`): read and write. */
function stored(id: number) {
  const keys = [`bench_result_${id}`, `bench_results_${id}`];
  const db = () => {
    const d = new DatabaseSync(join(dataDir, "rp-app.db"));
    d.exec("PRAGMA busy_timeout = 5000");
    return d;
  };
  return {
    read: () => {
      const d = db();
      try {
        return keys.map((k) => (d.prepare("SELECT value FROM settings WHERE key = ?").get(k) as { value: string } | undefined)?.value ?? null);
      } finally {
        d.close();
      }
    },
    write: (values: (string | null)[]) => {
      const d = db();
      try {
        keys.forEach((k, i) => {
          const v = values[i];
          if (v === null) d.prepare("DELETE FROM settings WHERE key = ?").run(k);
          else d.prepare("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(k, v);
        });
      } finally {
        d.close();
      }
    },
  };
}

async function openBench(page: Page, open: (hash: string) => Promise<void>) {
  await open("/settings/model");
  await page.getByRole("button", { name: "Beste Einstellung finden" }).click();
  const b = page.locator('[data-spot="benchmark"]');
  await expect(b.getByRole("heading", { name: "Einstellungen messen" })).toBeVisible();
  return b;
}

test("Benchmark-Karten: Text aus der gewählten Zeile, gleiche Zeilen eine Karte, CPU-Lauf ohne VRAM, nicht gemessene Stufen", async ({ page, open, theme }) => {
  test.skip(!dataDir, "needs RP_E2E_DATA");
  test.setTimeout(300_000);
  await settled();
  const p = await active();
  // A real run, unless an earlier one is stored.
  if (!(await bench(p.id)).result) {
    await api("POST", `/models/profiles/${p.id}/benchmark`, {});
    await expect.poll(async () => { const b = await bench(p.id); return !b.running && !!b.result; }, { timeout: 200_000, intervals: [2000] }).toBe(true);
    await settled();
  }
  const real = (await bench(p.id)).result!;
  const b = await openBench(page, open);

  // The real result (2k and 4k on the CPU): one card per chosen row, each saying what its row has.
  const rowsChosen = [...new Set(real.picks.map((x) => x.row))];
  await expect(b.locator(".bm-pick")).toHaveCount(rowsChosen.length);
  for (const card of await b.locator(".bm-pick").all()) {
    const purposes = (await card.getAttribute("data-purposes"))!.split(" ");
    const row = real.rows[real.picks.find((x) => x.purpose === purposes[0])!.row];
    for (const other of purposes) expect(real.picks.find((x) => x.purpose === other)!.row).toBe(real.picks.find((x) => x.purpose === purposes[0])!.row);
    await expect(card.locator(".bm-pick-facts")).toContainText(`${ctxShort(row.ctx)} Kontext · ${row.kv}`);
    if (row.ctx < 8192) await expect(card.locator(".help")).not.toContainText("8k");
  }
  if (real.gpu === null) await expect(b.locator(".x5-cond")).not.toContainText("Grafikspeicher");
  await b.locator(".bm-picks").scrollIntoViewIfNeeded();
  await shot(page, "bench-cards", theme);

  const store = stored(p.id);
  const saved = store.read();
  try {
    // All three chose the same row: one card that says so.
    const ok = real.rows.findIndex((r) => r.outcome === "ok");
    const fitting = real.rows.filter((r) => r.outcome === "ok").length;
    store.write([JSON.stringify({ ...real, picks: ["speed", "balanced", "context"].map((purpose) => ({ purpose, row: ok })) }), null]);
    await page.reload();
    await page.getByRole("button", { name: "Beste Einstellung finden" }).click();
    const one = b.locator(".bm-pick");
    await expect(one).toHaveCount(1);
    await expect(one.locator(".bm-pick-label")).toHaveText("Tempo · Ausgewogen · Kontext");
    await expect(one.locator(".help")).toHaveText(
      fitting === 1 ? "Nur diese Einstellung passte." : "Diese Einstellung ist hier zugleich die schnellste, die ausgewogenste und die mit dem meisten Kontext.",
    );
    await expect(one.getByRole("button", { name: "Tempo · Ausgewogen · Kontext setzen" })).toBeVisible();
    await one.scrollIntoViewIfNeeded();
    await shot(page, "bench-one-card", theme);

    // A CPU run that ran out of time: 8k stopped for RAM, 16k and 32k not reached; Tempo on 4k.
    const r = (ctx: number, kv: string, tg: number | null, outcome: string): Row => ({ ctx, kv, n_cpu_moe: 0, pp: tg === null ? null : 640.5, tg, overflow_mib: null, outcome, load_s: tg === null ? null : 1.8 });
    const rows = [r(2048, "f16", 21.4, "ok"), r(4096, "f16", 22.1, "ok"), r(4096, "q8_0", 19.7, "ok"), r(8192, "f16", null, "ram")];
    const cpu: Result = {
      ...real,
      rows,
      picks: [
        { purpose: "speed", row: 1 },
        { purpose: "balanced", row: 1 },
        { purpose: "context", row: 2 },
      ],
      gpu: null,
      conditions: { ram_available_mib: 9216, vram_free_mib: 12288, commit_free_mib: 20480 },
      unmeasured_stages: [16384, 32768],
      prompt_tokens: 512,
      gen_tokens: 32,
    };
    store.write([JSON.stringify(cpu), null]);
    await page.reload();
    await page.getByRole("button", { name: "Beste Einstellung finden" }).click();
    await expect(b.locator(".bm-pick")).toHaveCount(2);
    const speed = b.locator('.bm-pick[data-purpose="speed"]');
    await expect(speed.locator(".bm-pick-label")).toHaveText("Tempo · Ausgewogen");
    await expect(speed.locator(".help")).toHaveText("Diese Einstellung ist hier zugleich die schnellste und die ausgewogenste.");
    await expect(b.locator('.bm-pick[data-purpose="context"] .help')).toHaveText("So viel Kontext wie möglich, noch flüssig lesbar.");
    await expect(b.locator(".x5-cond")).toContainText("9,0 GB RAM verfügbar");
    await expect(b.locator(".x5-cond")).not.toContainText("Grafikspeicher");
    await expect(b.locator(".r2-unmeasured")).toHaveText("Aus Zeitgründen nicht gemessen: 16k und 32k Kontext.");
    await expect(b.locator(".x5-about")).toContainText("je Lauf 512 Tokens gelesen, 32 geschrieben");
    await b.locator(".x5-about").scrollIntoViewIfNeeded();
    await shot(page, "bench-cpu", theme);

    // Tempo alone on a 4k row while 8k was tried: says that 8k did not fit, not "ab 8k".
    store.write([JSON.stringify({ ...cpu, picks: [{ purpose: "speed", row: 1 }, { purpose: "context", row: 2 }] }), null]);
    await page.reload();
    await page.getByRole("button", { name: "Beste Einstellung finden" }).click();
    await expect(b.locator('.bm-pick[data-purpose="speed"] .help')).toHaveText("Die schnellsten Antworten. Ab 8k Kontext passte keine Einstellung.");
    // Only 2k and 4k measured: Tempo names how far the steps went.
    store.write([JSON.stringify({ ...cpu, rows: rows.slice(0, 3), picks: [{ purpose: "speed", row: 1 }, { purpose: "context", row: 2 }] }), null]);
    await page.reload();
    await page.getByRole("button", { name: "Beste Einstellung finden" }).click();
    await expect(b.locator('.bm-pick[data-purpose="speed"] .help')).toHaveText("Die schnellsten Antworten unter den gemessenen Stufen bis 4k Kontext.");
  } finally {
    store.write(saved);
  }
});
