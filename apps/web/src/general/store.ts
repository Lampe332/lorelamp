// The general settings (tab "Allgemein") as the server keeps them, the same on every device:
// which key sends, what the app opens with, when it asks first, how long an undo stays, how often
// it backs up by itself; and what belongs to the PC alone (starting with Windows, the data folder).
// The part other screens act on is mirrored in localStorage, so it holds before the first answer.

import { useSyncExternalStore } from "react";
import { api, errorText, postJson } from "../api";

export type SendKey = "enter" | "ctrl_enter";
export type StartPage = "home" | "last_chat" | "characters";
export type BackupRhythm = "daily" | "weekly" | "off";

export type General = {
  send_key: SendKey;
  start_page: StartPage;
  confirm_delete: boolean;
  confirm_rewind: boolean;
  /** How long "Rückgängig" stays on screen. */
  undo_secs: number;
  backup_rhythm: BackupRhythm;
  /** When the app backs up by itself next (ms; in the past: within minutes); null = never. */
  backup_next_ms: number | null;
  /** Where the app keeps its data on the PC; null on another device. */
  data_dir: string | null;
  /** Starting with Windows; null where the app cannot (a phone, a plain server). */
  autostart: { on: boolean; minimized: boolean } | null;
};

/** What can be changed with `saveGeneral`. */
export type GeneralEdit = Partial<Pick<General, "send_key" | "start_page" | "confirm_delete" | "confirm_rewind" | "undo_secs" | "backup_rhythm">>;

export const UNDO_CHOICES = [5, 8, 15, 30] as const;

const DEFAULTS: General = {
  send_key: "enter",
  start_page: "home",
  confirm_delete: false,
  confirm_rewind: false,
  undo_secs: 8,
  backup_rhythm: "daily",
  backup_next_ms: null,
  data_dir: null,
  autostart: null,
};

const KEY = "rp-general";

type State = { value: General; loaded: boolean; failed: boolean };

function cached(): General {
  try {
    const c = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<General> | null;
    return {
      ...DEFAULTS,
      send_key: c?.send_key === "ctrl_enter" ? "ctrl_enter" : "enter",
      start_page: c?.start_page === "last_chat" || c?.start_page === "characters" ? c.start_page : "home",
      confirm_delete: c?.confirm_delete === true,
      confirm_rewind: c?.confirm_rewind === true,
      undo_secs: UNDO_CHOICES.includes(c?.undo_secs as (typeof UNDO_CHOICES)[number]) ? (c?.undo_secs as number) : DEFAULTS.undo_secs,
    };
  } catch {
    return DEFAULTS;
  }
}

let state: State = { value: cached(), loaded: false, failed: false };
const listeners = new Set<() => void>();

function set(next: State) {
  state = next;
  if (next.loaded) {
    const { send_key, start_page, confirm_delete, confirm_rewind, undo_secs } = next.value;
    try {
      localStorage.setItem(KEY, JSON.stringify({ send_key, start_page, confirm_delete, confirm_rewind, undo_secs }));
    } catch {
      // Private windows and blocked storage: the server's answer is enough.
    }
  }
  for (const l of listeners) l();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};

/** The settings right now (for handlers; components use `useGeneral`). */
export const general = () => state.value;

export const useGeneral = () => useSyncExternalStore(subscribe, () => state);

let asked = 0;

/** Reads the settings from the server: at the start, and whenever a device changed them. */
export async function loadGeneral() {
  const mine = ++asked;
  try {
    const value = await api<General>("/general");
    if (mine === asked) set({ value, loaded: true, failed: false });
  } catch {
    if (mine === asked) set({ ...state, failed: true });
  }
}

/** A device changed the settings (the socket's "general" event). */
export const tellGeneralChanged = () => void loadGeneral();

/** Shows the change at once, then saves it; answers "" or what went wrong (then it is taken back). */
async function save(shown: General, run: () => Promise<General>): Promise<string> {
  const before = state;
  const mine = ++asked;
  set({ ...state, value: shown });
  try {
    const value = await run();
    if (mine === asked) set({ value, loaded: true, failed: false });
    return "";
  } catch (e) {
    if (mine === asked) set(before);
    return errorText(e);
  }
}

export const saveGeneral = (edit: GeneralEdit) => save({ ...state.value, ...edit }, () => postJson<General>("/general", edit, "PUT"));

export const saveAutostart = (on: boolean, minimized: boolean) =>
  save({ ...state.value, autostart: { on, minimized: on && minimized } }, () => postJson<General>("/general/autostart", { on, minimized }, "PUT"));

/** Shows the data folder in Windows Explorer; answers "" or what went wrong. */
export async function openDataFolder(): Promise<string> {
  try {
    await api("/general/open-data-folder", { method: "POST" });
    return "";
  } catch (e) {
    return errorText(e);
  }
}

/** Whether this key press in the message field sends the message (never on touch keyboards, where
 *  Enter is a line break and sending is the button; the caller checks that). */
export function sendsMessage(e: { key: string; shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }, mode: SendKey = state.value.send_key): boolean {
  if (e.key !== "Enter" || e.shiftKey) return false;
  return mode === "enter" || e.ctrlKey || e.metaKey;
}
