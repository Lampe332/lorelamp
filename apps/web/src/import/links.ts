// The pasted links of the import page and what became of each: waiting, loading, a preview to
// confirm, imported, or the way by file. Kept outside the page (like the file import in `job.ts`),
// so previews survive a look at another page. Links are fetched one after the other: a card site
// should not get a burst of requests from one paste.

import { useSyncExternalStore } from "react";
import { ApiError, postJson, type LinkPreview } from "../api";
import { logEvent } from "../log";
import { raw, t, type Key } from "../i18n";
import { knownReason } from "./reasons";

/** A web address the link import can try. */
export const isLink = (text: string) => /^https?:\/\/[^\s/$.?#][^\s]*$/i.test(text.trim());

/** Most links taken from one paste, and most kept in the list (the server keeps 24 previews). */
export const MAX_LINKS = 20;

/** The links in a pasted or typed text: one per line (or separated by spaces), each once. */
export function linksIn(text: string): string[] {
  const out: string[] = [];
  for (const part of text.split(/\s+/)) {
    // A link copied out of a sentence or a chat often drags a bracket or a comma along.
    const link = part.replace(/^[<(["']+/, "").replace(/[>)\]"',;]+$/, "");
    if (isLink(link) && !out.includes(link)) out.push(link);
  }
  return out;
}

/** Text that is a card in JSON (copied from a file or a site's "copy JSON" button). */
export function cardJson(text: string): boolean {
  const t = text.trim();
  if (!t.startsWith("{") || !t.endsWith("}")) return false;
  try {
    const v = JSON.parse(t) as { name?: unknown; data?: { name?: unknown } };
    return typeof (v.data?.name ?? v.name) === "string";
  } catch {
    return false;
  }
}

/** The address of a picture copied from a web page ("Bild kopieren" puts `<img src>` next to the
 *  pixels). The pixels on the clipboard have lost the card data; the file behind the address has not. */
export function copiedImageAddress(html: string): string | null {
  const src = /<img\b[^>]*\bsrc\s*=\s*["']([^"']+)["']/i.exec(html)?.[1]?.replace(/&amp;/g, "&");
  return src && isLink(src) ? src : null;
}

export type LinkAnswer = {
  status: "imported" | "duplicate" | "error" | "file_only";
  source: string;
  page_url?: string;
  reason_code?: string;
  character_id?: number;
  /** The other cards of a zip behind the link. */
  more?: { imported: number; duplicates: number; errors: number };
};
/** A fetched file without a card: the preview answers with `code`. */
type PreviewError = { status: "error"; code: string; source: string };
export type Preview = LinkPreview & { more_cards?: number };

export type LinkState =
  | { kind: "waiting" }
  | { kind: "loading" }
  /** `error`: the import of this preview failed; it can be tried again. */
  | { kind: "preview"; preview: Preview; importing: boolean; error?: string }
  | { kind: "answer"; answer: LinkAnswer }
  | { kind: "error"; text: string }
  /** The preview was confirmed too late (or twice); it can be loaded again. */
  | { kind: "expired" };

export type LinkItem = { key: number; url: string; state: LinkState };

const LINK_REASON: Record<string, Key> = {
  bad_url: "system.link.reason.bad_url",
  bad_link: "system.link.reason.bad_link",
  address_not_allowed: "system.link.reason.address_not_allowed",
  source_not_found: "system.link.reason.source_not_found",
  source_blocked: "system.link.reason.source_blocked",
  source_rate_limited: "system.link.reason.source_rate_limited",
  source_too_large: "system.link.reason.source_too_large",
  source_redirect: "system.link.reason.source_redirect",
  source_timeout: "system.link.reason.source_timeout",
  source_failed: "system.link.reason.source_failed",
  source_bad_answer: "system.link.reason.source_bad_answer",
  offline: "system.link.reason.offline",
};

/** Why a link did not work, in a sentence. */
export function linkReason(code: string | undefined): string | null {
  if (!code) return null;
  if (LINK_REASON[code]) return raw(LINK_REASON[code]);
  const known = knownReason(code);
  return known ? t("system.link.reason.sentence", { reason: known }) : null;
}

function linkError(e: unknown): string {
  if (e instanceof ApiError) {
    const known = linkReason(e.code);
    if (known) return known;
    // No link route on this server: an older app answers 404 or 405 without an error code.
    if (e.status === 404 || e.status === 405)
      return t("system.link.noRoute", { status: String(e.status) });
    if (e.status) return t("system.link.failedStatus", { status: String(e.status) });
  }
  return t("system.link.failed");
}

// ---------------------------------------------------------------------------------------------
// The list

let items: LinkItem[] = [];
let nextKey = 1;
const listeners = new Set<() => void>();
const set = (next: LinkItem[]) => {
  items = next;
  for (const l of listeners) l();
};
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
const patch = (key: number, state: LinkState) => set(items.map((i) => (i.key === key ? { ...i, state } : i)));
const find = (key: number) => items.find((i) => i.key === key);

export const useLinks = () => useSyncExternalStore(subscribe, () => items);

/** Still to show or to decide: everything that is not a finished answer or an error. */
const open = (i: LinkItem) => i.state.kind === "waiting" || i.state.kind === "loading" || i.state.kind === "preview";

let pumping = false;
/** Loads the waiting previews, one at a time. */
async function pump() {
  if (pumping) return;
  pumping = true;
  try {
    for (;;) {
      const next = items.find((i) => i.state.kind === "waiting");
      if (!next) return;
      patch(next.key, { kind: "loading" });
      logEvent("info", "import", "Link-Vorschau angefordert");
      let state: LinkState;
      try {
        const answer = await postJson<Preview | LinkAnswer | PreviewError>("/import/link/preview", { url: next.url });
        logEvent("info", "import", "Link-Vorschau da", { data: { status: answer.status, source: answer.source } });
        if (answer.status === "preview") state = { kind: "preview", preview: answer, importing: false };
        else if ("code" in answer) state = { kind: "answer", answer: { status: "error", source: answer.source, reason_code: answer.code } };
        else state = { kind: "answer", answer };
      } catch (e) {
        state = { kind: "error", text: linkError(e) };
      }
      // Discarded meanwhile: its answer is dropped.
      if (find(next.key)?.state.kind === "loading") patch(next.key, state);
    }
  } finally {
    pumping = false;
  }
}

/** Adds links and starts loading their previews. Finished entries (imported, errors, the way by
 *  file) leave the list; previews that still wait for a decision stay. Returns how many links did
 *  not fit. */
export function addLinks(urls: string[]): { added: number; left: number } {
  const kept = items.filter(open);
  const fresh = urls.filter((u, n) => urls.indexOf(u) === n && !kept.some((i) => i.url === u));
  const room = Math.max(0, MAX_LINKS - kept.length);
  const taken = fresh.slice(0, room);
  set([...kept, ...taken.map((url) => ({ key: nextKey++, url, state: { kind: "waiting" } as LinkState }))]);
  void pump();
  return { added: taken.length, left: fresh.length - taken.length };
}

/** Loads the preview of an entry again (after an error or an expired preview). */
export function reloadLink(key: number) {
  if (!find(key)) return;
  patch(key, { kind: "waiting" });
  void pump();
}

/** Imports a previewed card. */
export async function confirmLink(key: number): Promise<void> {
  const item = find(key);
  if (item?.state.kind !== "preview" || item.state.importing) return;
  const preview = item.state.preview;
  patch(key, { kind: "preview", preview, importing: true });
  try {
    const answer = await postJson<LinkAnswer>("/import/link/confirm", { preview_id: preview.preview_id });
    logEvent("info", "import", "Link-Import fertig", { data: { status: answer.status, source: answer.source } });
    if (find(key)) patch(key, { kind: "answer", answer: { ...answer, page_url: answer.page_url ?? preview.page_url } });
  } catch (e) {
    if (!find(key)) return;
    if (e instanceof ApiError && e.code === "preview_expired") patch(key, { kind: "expired" });
    else patch(key, { kind: "preview", preview, importing: false, error: linkError(e) });
  }
}

/** Imports every previewed card that is not in the library yet, one after the other. */
export async function confirmAllLinks(): Promise<void> {
  const keys = items.filter((i) => i.state.kind === "preview" && !i.state.importing && i.state.preview.duplicate_of === null).map((i) => i.key);
  for (const key of keys) await confirmLink(key);
}

export function discardLink(key: number) {
  set(items.filter((i) => i.key !== key));
}

export function clearLinks() {
  set([]);
}

// ---------------------------------------------------------------------------------------------
// The text of the link field (a single pasted link is shown there)

let draft = "";
const draftListeners = new Set<() => void>();
export function setDraft(text: string) {
  draft = text;
  for (const l of draftListeners) l();
}
const subscribeDraft = (l: () => void) => {
  draftListeners.add(l);
  return () => draftListeners.delete(l);
};
export const useDraft = () => useSyncExternalStore(subscribeDraft, () => draft);

/** Takes pasted or dropped links: one goes into the field too, several only into the list.
 *  Returns a note for the user when not all of them fit. */
export function takeLinks(urls: string[]): string {
  if (urls.length === 0) return "";
  const { left } = addLinks(urls);
  setDraft(urls.length === 1 ? urls[0] : "");
  if (left === 0) return "";
  return t("system.link.tooMany", { n: left, max: MAX_LINKS });
}
