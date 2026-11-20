// Activity log of this device: app start, clicks, page and tab changes, API calls, the socket,
// chat actions and script errors. Events are sent to the server in batches every 2 s and when
// the page is left; the server stores them with everything else that happens in the app.
//
// Privacy (fixed): never what is typed into a field, never story or card text, never the token.
// Click labels skip the parts of a control that show such text, and known names (characters,
// chat titles, the player's name) are blanked out.

import { apiUrl, token } from "./api";

declare const __APP_VERSION__: string;

export type LogLevel = "debug" | "info" | "warn" | "error";
type Extra = { data?: unknown; chat_id?: number; character_id?: number };
/** `repeat`/`last_ts`: equal events in a row, merged (see MERGE_MS). */
type UiEvent = { ts: number; level: LogLevel; category: string; message: string; repeat?: number; last_ts?: number } & Extra;

/** A short id of this browser, so the log tells two phones apart. */
export const clientId = (() => {
  const fresh = () => Math.random().toString(36).slice(2, 6);
  try {
    let id = localStorage.getItem("rp-client");
    if (!id) {
      id = fresh();
      localStorage.setItem("rp-client", id);
    }
    return id;
  } catch {
    return fresh();
  }
})();

const FLUSH_MS = 2000;
const MAX_QUEUE = 2000;
let queue: UiEvent[] = [];
let privateTerms: string[] = [];

/** Names that must not appear in the log (characters, chat titles, the player's name). */
export function setPrivateTerms(terms: string[]) {
  privateTerms = [...new Set(terms.map((t) => t.trim()).filter((t) => t.length >= 2))].sort((a, b) => b.length - a.length);
}

const scrub = (text: string) => privateTerms.reduce((out, term) => out.split(term).join("‹Name›"), text);

/** An event equal to the one queued before it, at most this much later, raises its counter instead. */
const MERGE_MS = 60_000;

/** Answers at least this slow are logged even when they succeed (as on the server, `reqlog.rs`). */
const SLOW_MS = 1000;

/** An API call that succeeded quickly says nothing; only failures, missing answers and slow calls are kept. */
function quickApiCall(category: string, data: unknown): boolean {
  if (category !== "api" || typeof data !== "object" || data === null) return false;
  const { status, ms } = data as { status?: unknown; ms?: unknown };
  return typeof status === "number" && typeof ms === "number" && status > 0 && status < 400 && ms < SLOW_MS;
}

const sameEvent = (a: UiEvent, b: UiEvent) =>
  a.level === b.level &&
  a.category === b.category &&
  a.message === b.message &&
  a.chat_id === b.chat_id &&
  a.character_id === b.character_id &&
  JSON.stringify(a.data) === JSON.stringify(b.data);

export function logEvent(level: LogLevel, category: string, message: string, extra: Extra = {}) {
  if (quickApiCall(category, extra.data)) return;
  const ev: UiEvent = { ts: Date.now(), level, category, message: scrub(message).slice(0, 300), ...extra };
  const last = queue[queue.length - 1];
  if (last && sameEvent(last, ev) && ev.ts - (last.last_ts ?? last.ts) <= MERGE_MS) {
    last.repeat = (last.repeat ?? 1) + 1;
    last.last_ts = ev.ts;
    return;
  }
  queue.push(ev);
  if (queue.length > MAX_QUEUE) queue = queue.slice(-MAX_QUEUE);
}

/** A path without its query (the query may carry the token). */
export const bare = (url: string) => url.split("?")[0];

let sending = false;

/** Sends what is queued. On the way out (`leaving`) in small keepalive requests that survive the page. */
export async function flushLogs(leaving = false): Promise<void> {
  if (queue.length === 0 || (sending && !leaving)) return;
  const post = (events: UiEvent[]) =>
    fetch(apiUrl("/logs"), {
      method: "POST",
      keepalive: leaving,
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json", "X-RP-Client": clientId },
      // Belt and braces: whatever slipped into an event, the token never leaves in it.
      body: token ? JSON.stringify({ events }).split(token).join("<token>") : JSON.stringify({ events }),
    });
  if (leaving) {
    // Keepalive bodies are limited (64 KB in flight), so send small pieces.
    while (queue.length) void post(queue.splice(0, 40)).catch(() => {});
    return;
  }
  const batch = queue.splice(0, 500);
  sending = true;
  try {
    const res = await post(batch);
    if (!res.ok) throw new Error(String(res.status));
  } catch {
    queue = [...batch, ...queue].slice(-MAX_QUEUE); // try again with the next round
  } finally {
    sending = false;
  }
}

// ---------------------------------------------------------------- clicks

/** Real controls only (buttons, links, switches, choices). A click on a surface, on text or into a
 *  text field is not logged; a click on a label reaches its switch as a click of its own. */
const CONTROL = [
  "button",
  "a[href]",
  "summary",
  "select",
  "input[type=checkbox]",
  "input[type=radio]",
  "input[type=button]",
  "input[type=submit]",
  ...["button", "tab", "switch", "checkbox", "radio", "menuitem", "menuitemcheckbox", "menuitemradio", "option"].map((r) => `[role=${r}]`),
].join(", ");
/** Parts of a control that show story, card or typed text. */
const PRIVATE = "blockquote, .prose, .body, .l1, .l2, .r, .who, .chat-title, .recent span, h1, textarea, input, select";

function ownText(el: Element): string {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let out = "";
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const hidden = n.parentElement?.closest(PRIVATE);
    if (hidden && el.contains(hidden)) continue;
    out += ` ${n.textContent ?? ""}`;
  }
  return out.replace(/\s+/g, " ").trim();
}

/** A text field is named by its label, never by what is in it. */
function fieldName(el: Element): string {
  const field = el.matches("input, textarea, select") ? el : el.querySelector("input, textarea, select");
  if (!field) return "";
  // The visible label of a form field (`<label class="field"><span class="field-label">Tags</span>…`) is fixed UI text.
  const label = field.closest("label")?.querySelector(".field-label")?.firstChild?.textContent?.trim();
  const name =
    field.getAttribute("aria-label") || field.getAttribute("placeholder") || label || field.getAttribute("name") || field.tagName.toLowerCase();
  return `Feld ${name}`;
}

function labelOf(el: Element): string {
  const label =
    el.getAttribute("data-log-label") ||
    el.getAttribute("title") ||
    el.getAttribute("aria-label") ||
    ownText(el) ||
    fieldName(el) ||
    `<${el.tagName.toLowerCase()}>`;
  const short = scrub(label);
  return short.length > 60 ? `${short.slice(0, 60)}…` : short;
}

function onClick(e: MouseEvent) {
  if (!e.isTrusted) return; // the app's own synthetic clicks (the export download)
  const el = (e.target as Element | null)?.closest?.(CONTROL);
  if (!el) return;
  const href = el.getAttribute("href");
  logEvent("info", "click", `Klick: „${labelOf(el)}“`, {
    data: { route: location.hash || "#/", tag: el.tagName.toLowerCase(), ...(href ? { href: bare(href) } : {}) },
  });
}

// ---------------------------------------------------------------- pages

let lastHash = location.hash || "#/";

/** The screen a hash belongs to, without its tab: `#/char/3/chats` -> `#/char/3`, `#/settings/logs` -> `#/settings`. */
const screenOf = (hash: string) => hash.replace(/^(#\/char\/\d+)\/(chats|settings)$/, "$1").replace(/^#\/settings\/\w+$/, "#/settings");

function onHash() {
  const to = location.hash || "#/";
  const from = lastHash;
  lastHash = to;
  if (to === from) return;
  const tab = screenOf(from) === screenOf(to);
  logEvent("info", tab ? "tab" : "nav", `${tab ? "Tab" : "Seite"}: ${to}`, { data: { from, to } });
}

// ---------------------------------------------------------------- start

let started = false;

/** Logs the start of the app and watches clicks, pages, errors and leaving. Call once. */
export function startLogging() {
  if (started) return;
  started = true;
  const script = (() => {
    try {
      return new URL(import.meta.url).pathname.split("/").pop() ?? "";
    } catch {
      return "";
    }
  })();
  logEvent("info", "app", `App gestartet (Version ${__APP_VERSION__})`, {
    data: {
      version: __APP_VERSION__,
      build: script,
      user_agent: navigator.userAgent,
      screen: `${screen.width}x${screen.height}@${Math.round(devicePixelRatio * 100) / 100}`,
      viewport: `${innerWidth}x${innerHeight}`,
      language: navigator.language,
      time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      route: lastHash,
      client: clientId,
    },
  });

  document.addEventListener("click", onClick, true);
  window.addEventListener("hashchange", onHash);
  window.addEventListener("error", (e) => {
    const err = e.error as Error | undefined;
    logEvent("error", "error", `Skriptfehler: ${e.message}`, {
      data: { file: bare(e.filename ?? ""), line: e.lineno, column: e.colno, stack: err?.stack?.slice(0, 3000) },
    });
  });
  window.addEventListener("unhandledrejection", (e) => {
    const r = e.reason as unknown;
    const message = r instanceof Error ? r.message : String(r);
    logEvent("error", "error", `Unbehandelter Fehler: ${message}`, { data: { stack: r instanceof Error ? r.stack?.slice(0, 3000) : undefined } });
  });
  document.addEventListener("visibilitychange", () => {
    const hidden = document.visibilityState === "hidden";
    logEvent("debug", "app", hidden ? "App im Hintergrund" : "App wieder im Vordergrund");
    if (hidden) void flushLogs(true);
  });
  window.addEventListener("pagehide", () => {
    logEvent("info", "app", "Seite verlassen");
    void flushLogs(true);
  });
  window.setInterval(() => void flushLogs(), FLUSH_MS);
}
