// One import at a time, kept outside the page: it keeps running while the user looks elsewhere, and
// the import page shows it again on return. Files go up in packages (at most 50 files or 64 MB per
// request, one after the other, all under one job id), so a cancel takes effect between packages
// and a large folder never becomes one huge request. Progress comes from the server's socket events
// (`{"type":"import",…}`) plus the packages already answered.

import { useSyncExternalStore } from "react";
import { api, ApiError, postJson, tzMinutes, type StInstall } from "../api";
import { logEvent } from "../log";

/** One file (or one zip entry, one persona of a backup) as the server answered it. */
export type Row = {
  file: string;
  status: "imported" | "duplicate" | "error";
  code?: string;
  character_id?: number;
  persona_id?: number;
  /** A chat taken over from SillyTavern (with its `character_id`). */
  chat_id?: number;
  /** Such a chat: lines SillyTavern hid or marked as system lines, which were left out. */
  system_skipped?: number;
};

type Answer = { results: Row[]; imported: number; duplicates: number; errors: number };

/** A file the user picked or dropped; `path` keeps the folders below the chosen one. */
export type Picked = { file: File; path: string };

export type Job = {
  id: string;
  state: "running" | "done" | "cancelled" | "failed";
  /** Files that go to the server. */
  total: number;
  /** Files whose package the server has answered. */
  sent: number;
  /** Share of the package now on its way that the server has read (0..1, from the socket). */
  live: number;
  batch: number;
  batches: number;
  /** Files in the package on its way. */
  current: number;
  /** The user asked to stop; the package on its way still finishes. */
  stopping: boolean;
  rows: Row[];
  /** Why the import ended early (a readable sentence). */
  error?: string;
  /** A SillyTavern import: how many of each were chosen, and whether the server has stored the
   *  cards (its last progress event; the chats are stored after it). */
  st?: { cards: number; personas: number; chats: number; cardsDone: boolean };
};

export const BATCH_FILES = 50;
export const BATCH_BYTES = 64 << 20;
/** Largest single file the server takes (rp_core::formats::MAX_INPUT). */
export const MAX_FILE = 128 << 20;

const IMPORT_EVENT = "rp-import";

let job: Job | null = null;
const listeners = new Set<() => void>();
const set = (next: Job | null) => {
  job = next;
  for (const l of listeners) l();
};
const patch = (p: Partial<Job>) => job && set({ ...job, ...p });

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

/** The current (or last) import; null before the first one. */
export const useImportJob = () => useSyncExternalStore(subscribe, () => job);

/** Socket events of an import, passed on by the app's one socket (see useChatSocket). */
export type ImportEvent = { type: "import"; job: string; done: number; total: number; finished: boolean };
export const forwardImportEvent = (ev: ImportEvent) => window.dispatchEvent(new CustomEvent<ImportEvent>(IMPORT_EVENT, { detail: ev }));

// ---------------------------------------------------------------------------------------------
// What goes where

/** Files no import wants: folder thumbnails and the like of Windows and macOS. */
const junk = (name: string) => name.startsWith(".") || /^(thumbs\.db|desktop\.ini)$/i.test(name);

const segments = (p: Picked) => p.path.split(/[\\/]/).filter(Boolean);

export type Plan = { cards: Picked[]; personas: Picked[]; sillyTavern: boolean };

/** Splits files into cards and persona files. A SillyTavern data folder (`characters/` beside a
 *  `settings.json`) gives only what can be taken over: the cards directly in `characters/` (not the
 *  expression pictures in its subfolders) and `settings.json` with `User Avatars/`; chats, lorebooks
 *  and backups stay where they are. Elsewhere, `personas_*.json` (SillyTavern's persona backup) goes
 *  to the personas, everything else to the cards. */
export function planImport(files: Picked[], personasOnly = false): Plan {
  const kept = files.filter((p) => !junk(p.file.name));
  if (personasOnly) return { cards: [], personas: kept, sillyTavern: false };
  const parent = (p: Picked) => segments(p).at(-2) ?? "";
  const stCards = kept.filter((p) => parent(p) === "characters");
  const stSettings = kept.filter((p) => p.file.name === "settings.json" && segments(p).length >= 2);
  if (stCards.length > 0 && stSettings.length > 0) {
    const avatars = kept.filter((p) => parent(p) === "User Avatars");
    return { cards: stCards, personas: [...stSettings, ...avatars], sillyTavern: true };
  }
  const personaBackup = (p: Picked) => /^personas_.*\.json$/i.test(p.file.name);
  return { cards: kept.filter((p) => !personaBackup(p)), personas: kept.filter(personaBackup), sillyTavern: false };
}

/** Packages of at most `BATCH_FILES` files and `BATCH_BYTES` (a larger file goes alone). */
export function packages(files: Picked[]): Picked[][] {
  const out: Picked[][] = [];
  let cur: Picked[] = [];
  let bytes = 0;
  for (const p of files) {
    if (cur.length > 0 && (cur.length >= BATCH_FILES || bytes + p.file.size > BATCH_BYTES)) {
      out.push(cur);
      cur = [];
      bytes = 0;
    }
    cur.push(p);
    bytes += p.file.size;
  }
  if (cur.length) out.push(cur);
  return out;
}

const newId = () => `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

const form = (files: Picked[]) => {
  const f = new FormData();
  for (const p of files) f.append("file", p.file, p.path);
  return f;
};

const codeOf = (e: unknown) => (e instanceof ApiError ? e.code : "unknown");

// ---------------------------------------------------------------------------------------------
// Running

/** Starts an import (cards, persona files or both, as `planImport` splits them). Returns false when
 *  another one is still running or there is nothing to send. */
export function startImport(files: Picked[], opts: { personasOnly?: boolean } = {}): boolean {
  if (job?.state === "running") return false;
  const plan = planImport(files, opts.personasOnly);
  const tooBig = (p: Picked) => p.file.size > MAX_FILE;
  const refused: Row[] = [...plan.cards, ...plan.personas].filter(tooBig).map((p) => ({ file: p.path, status: "error", code: "too_large" }));
  const cards = packages(plan.cards.filter((p) => !tooBig(p)));
  const personas = plan.personas.filter((p) => !tooBig(p));
  const total = cards.reduce((n, b) => n + b.length, 0) + personas.length;
  if (total === 0 && refused.length === 0) return false;
  const id = newId();
  set({ id, state: "running", total, sent: 0, live: 0, current: 0, batch: 0, batches: cards.length + (personas.length ? 1 : 0), stopping: false, rows: refused });
  void run(id, cards, personas, plan.sillyTavern);
  return true;
}

/** What to take over from a SillyTavern installation (ids as `GET /import/sillytavern/contents` gives them). */
export type StSelection = { characters: string[]; personas: string[]; chats: string[] };

/** Takes over exactly the chosen cards, personas and chats of a SillyTavern installation the server
 *  found: the server reads them itself (one request, progress of the cards over the socket like a
 *  file import). Returns the job's id, or null while another import runs or nothing is chosen. */
export function startSillyTavernImport(install: StInstall, pick: StSelection): string | null {
  if (job?.state === "running") return null;
  const st = { cards: pick.characters.length, personas: pick.personas.length, chats: pick.chats.length, cardsDone: false };
  const total = st.cards + st.personas + st.chats;
  if (total === 0) return null;
  const id = newId();
  set({ id, state: "running", total, sent: 0, live: 0, current: st.cards, batch: 1, batches: 1, stopping: false, rows: [], st });
  void runSillyTavern(id, install, pick);
  return id;
}

async function runSillyTavern(id: string, install: StInstall, pick: StSelection) {
  const mine = () => job?.id === id;
  const onEvent = (e: Event) => {
    const ev = (e as CustomEvent<ImportEvent>).detail;
    if (ev.job !== id || !mine() || ev.total <= 0) return;
    patch({ live: Math.min(1, ev.done / ev.total), ...(ev.finished && job?.st ? { st: { ...job.st, cardsDone: true } } : {}) });
  };
  window.addEventListener(IMPORT_EVENT, onEvent);
  logEvent("info", "import", "SillyTavern-Import gestartet", {
    data: { characters: pick.characters.length, personas: pick.personas.length, chats: pick.chats.length, old_layout: install.user === null },
  });
  let rows: Row[];
  let error: string | undefined;
  try {
    // SillyTavern writes some dates in local time; the server needs this device's offset for them.
    const body = { path: install.path, user: install.user ?? undefined, job: id, tz: tzMinutes(), ...pick };
    rows = (await postJson<Answer>("/import/sillytavern", body)).results;
  } catch (e) {
    const code = codeOf(e);
    if (code === "offline") error = "offline";
    rows = [{ file: install.path, status: "error", code }];
  } finally {
    window.removeEventListener(IMPORT_EVENT, onEvent);
  }
  if (!mine() || !job) return;
  const n = (s: Row["status"]) => rows.filter((r) => r.status === s).length;
  logEvent(error || n("error") ? "warn" : "info", "import", "SillyTavern-Import fertig", {
    data: { imported: n("imported"), duplicates: n("duplicate"), errors: n("error"), ...(error ? { error } : {}) },
  });
  // The server counts what it really read; the estimate from the search gives way to it.
  const read = error ? 0 : rows.length;
  patch({ state: error ? "failed" : "done", rows, total: Math.max(read, 1), sent: read, live: 0, current: 0, error });
}

/** Stops after the package that is on its way. */
export function cancelImport() {
  if (job?.state === "running") patch({ stopping: true });
}

/** Forgets the finished import (its list goes away). */
export function clearImport() {
  if (job && job.state !== "running") set(null);
}

async function run(id: string, cards: Picked[][], personas: Picked[], sillyTavern: boolean) {
  const mine = () => job?.id === id;
  const onEvent = (e: Event) => {
    const ev = (e as CustomEvent<ImportEvent>).detail;
    if (ev.job === id && mine() && ev.total > 0) patch({ live: Math.min(1, ev.done / ev.total) });
  };
  window.addEventListener(IMPORT_EVENT, onEvent);
  logEvent("info", "import", "Import gestartet", {
    data: { files: (job?.total ?? 0) + (job?.rows.length ?? 0), packages: cards.length, personas: personas.length, sillytavern: sillyTavern },
  });
  const steps: { path: string; files: Picked[] }[] = [
    ...cards.map((files) => ({ path: `/import?job=${id}`, files })),
    ...(personas.length ? [{ path: "/personas/import", files: personas }] : []),
  ];
  let error: string | undefined;
  try {
    for (const [i, step] of steps.entries()) {
      if (!mine() || job?.stopping) break;
      patch({ batch: i + 1, live: 0, current: step.files.length });
      let rows: Row[];
      try {
        rows = (await api<Answer>(step.path, { method: "POST", body: form(step.files) })).results;
      } catch (e) {
        const code = codeOf(e);
        // Without a connection the rest cannot go either; any other refusal only hits this package.
        if (code === "offline") {
          error = "offline";
          break;
        }
        rows = step.files.map((p) => ({ file: p.path, status: "error", code }));
      }
      if (!mine() || !job) return;
      patch({ rows: [...job.rows, ...rows], sent: job.sent + step.files.length, live: 0, current: 0 });
    }
  } finally {
    window.removeEventListener(IMPORT_EVENT, onEvent);
  }
  if (!mine() || !job) return;
  const state = error ? "failed" : job.stopping && job.sent < job.total ? "cancelled" : "done";
  const n = (s: Row["status"]) => job!.rows.filter((r) => r.status === s).length;
  logEvent(error || n("error") ? "warn" : "info", "import", "Import fertig", {
    data: { state, imported: n("imported"), duplicates: n("duplicate"), errors: n("error"), ...(error ? { error } : {}) },
  });
  patch({ state, stopping: false, live: 0, error });
}

// ---------------------------------------------------------------------------------------------
// Reading what was dropped

type Entry = FileSystemEntry;

const fileOf = (e: FileSystemFileEntry) => new Promise<File>((ok, fail) => e.file(ok, fail));

/** All entries of a folder: `readEntries` hands them out in portions until it returns none. */
async function children(dir: FileSystemDirectoryEntry): Promise<Entry[]> {
  const reader = dir.createReader();
  const out: Entry[] = [];
  for (;;) {
    const part = await new Promise<Entry[]>((ok, fail) => reader.readEntries(ok, fail));
    if (part.length === 0) return out;
    out.push(...part);
  }
}

async function walk(entry: Entry, out: Picked[]) {
  if (entry.isFile) {
    const file = await fileOf(entry as FileSystemFileEntry).catch(() => null);
    if (file) out.push({ file, path: entry.fullPath.replace(/^\//, "") || file.name });
  } else if (entry.isDirectory) {
    for (const child of await children(entry as FileSystemDirectoryEntry).catch(() => [])) await walk(child, out);
  }
}

/** Files and whole folders (with everything below them) from a drop. The entries must be taken
 *  while the drop event runs; reading them may come later. */
export async function droppedFiles(dt: DataTransfer): Promise<Picked[]> {
  const entries = [...dt.items].map((i) => (i.kind === "file" ? i.webkitGetAsEntry?.() : null));
  const loose = [...dt.files];
  if (!entries.some(Boolean)) return loose.map((file) => ({ file, path: file.name }));
  const out: Picked[] = [];
  for (const e of entries) if (e) await walk(e, out);
  return out;
}

/** Files from a file field; with `webkitdirectory` they carry their path below the chosen folder. */
export const pickedFiles = (list: FileList | null): Picked[] =>
  [...(list ?? [])].map((file) => ({ file, path: file.webkitRelativePath || file.name }));
