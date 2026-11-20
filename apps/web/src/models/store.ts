// The download list, shared by every place that shows it (the sidebar's badge, the panel in the tabs
// "Vorschläge" and "Modell"): loaded once, then kept up to date by the socket's "download" events;
// "catalog" events tell the tab "Vorschläge" that the live check is done. Plus the words and number
// formats these places share.

import { useEffect, useSyncExternalStore } from "react";
import { api, ApiError, ERROR_TEXT, postJson } from "../api";
import { dateFormat, num, t } from "../i18n";
import type { Download, DownloadEvent } from "./types";
import { withUnit } from "./words";

type Snapshot = { list: Download[] | null; folder: string };

let snap: Snapshot = { list: null, folder: "" };
const listeners = new Set<() => void>();
let loading: Promise<void> | null = null;

function set(next: Snapshot) {
  snap = next;
  for (const l of listeners) l();
}

export function loadDownloads(): Promise<void> {
  loading ??= api<{ downloads: Download[]; folder: string }>("/models/downloads")
    .then((r) => set({ list: r.downloads, folder: r.folder }))
    .catch(() => {
      // offline: the next event or visit asks again
    })
    .finally(() => {
      loading = null;
    });
  return loading;
}

/** A socket event: one download changed, or left the list. */
export function applyDownloadEvent(ev: DownloadEvent) {
  const list = snap.list;
  if (!list) return void loadDownloads();
  if (ev.state === "removed") return set({ ...snap, list: list.filter((d) => d.id !== ev.id) });
  const old = list.find((d) => d.id === ev.id);
  if (!old) return void loadDownloads(); // added on another device
  const { type: _type, ...fields } = ev;
  set({ ...snap, list: list.map((d) => (d.id === ev.id ? ({ ...d, ...fields } as Download) : d)) });
}

const CATALOG = "rp-catalog-news";
/** The socket's "catalog" event: the live check of the catalog is done. */
export const tellCatalogNews = () => window.dispatchEvent(new Event(CATALOG));
export function useCatalogNews(fn: () => void) {
  useEffect(() => {
    window.addEventListener(CATALOG, fn);
    return () => window.removeEventListener(CATALOG, fn);
  }, [fn]);
}

function subscribe(l: () => void) {
  listeners.add(l);
  if (snap.list === null) void loadDownloads();
  return () => listeners.delete(l);
}

export function useDownloads(): Snapshot {
  return useSyncExternalStore(subscribe, () => snap);
}

/** Downloads still to finish (running, waiting or paused). */
export const openDownloads = (list: Download[] | null) => (list ?? []).filter((d) => d.state !== "done" && d.state !== "failed");

export type StartBody = { repo: string; file?: string; files?: string[]; sha256?: string; size?: number };

/** Starts a download; the answer goes into the list at once (the socket event may come first). */
export async function startDownload(body: StartBody): Promise<Download> {
  const d = await postJson<Download>("/models/downloads", body);
  const list = snap.list ?? [];
  set({ ...snap, list: list.some((x) => x.id === d.id) ? list : [...list, d] });
  return d;
}

export async function downloadAction(id: string, action: "pause" | "resume" | "remove") {
  if (action === "remove") await api(`/models/downloads/${encodeURIComponent(id)}`, { method: "DELETE" });
  else {
    const d = await api<Download>(`/models/downloads/${encodeURIComponent(id)}/${action}`, { method: "POST" });
    set({ ...snap, list: (snap.list ?? []).map((x) => (x.id === d.id ? d : x)) });
  }
}

/** The download of this repo file, if one is in the list. */
export const downloadOf = (list: Download[] | null, repo: string, file: string) =>
  (list ?? []).find((d) => d.repo === repo && d.files.some((f) => f.path === file));

/** The download errors this area has words for (`models.dl.err.<code>`); any other code reads as the
 *  frame's "unknown". */
const ERRORS = [
  "gated",
  "repo_missing",
  "file_missing",
  "bad_url",
  "bad_repo",
  "bad_file",
  "bad_query",
  "exists",
  "already_downloading",
  "no_space",
  "hf_unreachable",
  "rate_limited",
  "hash_mismatch",
  "size_mismatch",
  "network",
  "file_error",
] as const;

export const dlErrorText = (code: string | null | undefined) => {
  const own = ERRORS.find((c) => c === code);
  return own ? t(`models.dl.err.${own}`) : ERROR_TEXT.unknown;
};
export const errorOf = (e: unknown) => dlErrorText(e instanceof ApiError ? e.code : null);

const KB = 1024;
const MB = KB * KB;
const GB = MB * KB;

/** Sizes as Windows shows them (1 GB = 2^30 bytes): 18 687 062 976 → "17,4 GB", 47 227 552 → "45 MB". */
export function size(bytes: number): string {
  if (bytes >= GB) return withUnit(bytes / GB, "gigabyte", 1);
  if (bytes >= MB) return withUnit(bytes / MB, "megabyte", 0);
  // Windows' "KB" (Intl would write the SI "kB").
  return t("models.unit.kb", { v: num(Math.round(Math.max(1, bytes / KB))) });
}

/** Whole GB of a memory size (VRAM, RAM). */
export const memGb = (bytes: number) => Math.round(bytes / GB);

export { FIT_NAME, gb, TIER_NAME, TIERS } from "./words";

export function speedText(bps: number): string {
  return bps >= MB ? withUnit(bps / MB, "megabyte-per-second", 1) : t("models.unit.kbs", { v: num(Math.round(bps / KB)) });
}

/** "noch ca. 6 Min." from the bytes left and the speed. */
export function etaText(left: number, bps: number): string | null {
  if (bps < 1000 || left <= 0) return null;
  const s = left / bps;
  if (s < 60) return t("models.dl.etaUnder");
  if (s < 3600) return t("models.dl.eta", { time: withUnit(Math.ceil(s / 60), "minute", 0) });
  return t("models.dl.eta", { time: withUnit(s / 3600, "hour", 1) });
}

/** "2026-11-16" → "16.11.2026" (de), "11/16/2026" (en); anything else stays as it is. */
export const dateShort = (iso: string) => {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso);
  return m ? dateFormat({ day: "2-digit", month: "2-digit", year: "numeric", timeZone: "UTC" }).format(Date.UTC(+m[1], +m[2] - 1, +m[3])) : iso;
};
