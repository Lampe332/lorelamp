// "The answer is ready" while the app is in the background: a dot in front of the page title until
// the user comes back, a short quiet tone made with WebAudio, and a system notification when the
// browser already allows them. The choice lives on this device only (localStorage), like a sound
// setting of a single phone or PC; the server knows nothing of it.

import { useSyncExternalStore } from "react";
import { t } from "./i18n";

export type NotifyPrefs = {
  /** Title mark, and a system notification when the browser allows them. */
  on: boolean;
  /** A short quiet tone as well. */
  sound: boolean;
};

const KEY = "rp-notify";
const DEFAULTS: NotifyPrefs = { on: true, sound: true };
const MARK = "● ";

let prefs = read();
const listeners = new Set<() => void>();

function read(): NotifyPrefs {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<NotifyPrefs> | null;
    return {
      on: typeof saved?.on === "boolean" ? saved.on : DEFAULTS.on,
      sound: typeof saved?.sound === "boolean" ? saved.sound : DEFAULTS.sound,
    };
  } catch {
    return DEFAULTS;
  }
}

export const notifyPrefs = () => prefs;

export function setNotifyPrefs(next: Partial<NotifyPrefs>) {
  prefs = { ...prefs, ...next };
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Private windows and blocked storage: the choice holds until the page is closed.
  }
  for (const l of listeners) l();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};

export const useNotifyPrefs = () => useSyncExternalStore(subscribe, notifyPrefs);

/** What the browser says about system notifications: "unsupported" where there are none (e.g. iOS
 *  outside a home-screen app). */
export function notificationState(): NotificationPermission | "unsupported" {
  return typeof Notification === "undefined" ? "unsupported" : Notification.permission;
}

/** Asks for system notifications, only when the user has just switched the hint on and the
 *  browser has not been asked yet. */
export async function askForNotifications(): Promise<NotificationPermission | "unsupported"> {
  if (notificationState() !== "default") return notificationState();
  try {
    return await Notification.requestPermission();
  } catch {
    return notificationState();
  }
}

/** The page is hidden or the window does not have the focus: the user would miss the answer. */
const away = () => document.hidden || !document.hasFocus();

function clearMark() {
  if (away()) return;
  if (document.title.startsWith(MARK)) document.title = document.title.slice(MARK.length);
}
document.addEventListener("visibilitychange", clearMark);
window.addEventListener("focus", clearMark);

let audio: AudioContext | null = null;

/** Two soft notes, a fifth apart, about a third of a second. */
function chime() {
  try {
    audio ??= new AudioContext();
    const ctx = audio;
    void ctx.resume().catch(() => {});
    const start = ctx.currentTime + 0.02;
    [660, 990].forEach((hz, i) => {
      const t = start + i * 0.12;
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.value = hz;
      gain.gain.setValueAtTime(0, t);
      gain.gain.linearRampToValueAtTime(0.05, t + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.22);
      osc.connect(gain).connect(ctx.destination);
      osc.start(t);
      osc.stop(t + 0.24);
    });
  } catch {
    // No audio output or no WebAudio: the title mark still shows.
  }
}

/** An answer has just been finished. `name`: the character, when its chat is the one on screen. */
export function answerReady(chatId: number, name: string | null) {
  if (!prefs.on || !away()) return;
  if (!document.title.startsWith(MARK)) document.title = MARK + document.title;
  if (prefs.sound) chime();
  if (notificationState() !== "granted") return;
  try {
    const n = new Notification(name ?? "Lorelamp", { body: t("chat.notify.done"), tag: "rp-answer", silent: true });
    n.onclick = () => {
      window.focus();
      location.hash = `#/chat/${chatId}`;
      n.close();
    };
  } catch {
    // Some browsers only allow notifications from a service worker; the title mark is enough there.
  }
}
