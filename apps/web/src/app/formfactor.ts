// Which of the app's three layouts this window gets. One app, three form factors:
//   phone    width < 768
//   tablet   width 768–1366 AND a finger as the main pointer (`pointer: coarse`) or the Android app
//            (`shell=android` in the address or kept for the tab, see phone/shell.ts)
//   desktop  everything else: a window on the PC with a mouse stays exactly as it was
// The answer is also `data-form` on <html>, so CSS can follow it (`html[data-form="tablet"] …`).
// Next to it stands `data-turn`: "upright" or "sideways", how the device is held.
// Both follow turning the device and resizing; a change is attributes and CSS, nothing remounts.

import { useSyncExternalStore } from "react";
import { inApp } from "../phone/shell";

export type FormFactor = "phone" | "tablet" | "desktop";

/** The widest tablet window (an iPad Pro 12.9" held sideways). */
export const TABLET_MAX = 1366;

const phoneWide = matchMedia("(max-width: 767px)");
const tabletWide = matchMedia(`(max-width: ${TABLET_MAX}px)`);
const finger = matchMedia("(pointer: coarse)");

function read(): FormFactor {
  if (phoneWide.matches) return "phone";
  if (tabletWide.matches && (inApp || finger.matches)) return "tablet";
  return "desktop";
}

// How the device is held. CSS `orientation` will not do: an open on-screen keyboard takes so much
// of an upright window's height that it becomes wider than tall, and the layout would jump while
// typing. So the height counts as the tallest this width has had (a keyboard only ever takes
// height away), and as the screen's own height when the window fills the screen's width.
let turnWidth = window.innerWidth;
let tallest = window.innerHeight;
function readTurn(): "upright" | "sideways" {
  const w = window.innerWidth;
  const h = window.innerHeight;
  if (w !== turnWidth) {
    turnWidth = w;
    tallest = h;
  } else if (h > tallest) tallest = h;
  const full = Math.abs(window.screen.width - w) < 2 ? window.screen.height : 0;
  return w > Math.max(tallest, full) ? "sideways" : "upright";
}

let current = read();
document.documentElement.dataset.form = current;
document.documentElement.dataset.turn = readTurn();
const listeners = new Set<() => void>();

function check() {
  document.documentElement.dataset.turn = readTurn();
  const next = read();
  if (next === current) return;
  current = next;
  document.documentElement.dataset.form = next;
  for (const fn of listeners) fn();
}
for (const q of [phoneWide, tabletWide, finger]) q.addEventListener("change", check);
window.addEventListener("resize", check);

function subscribe(fn: () => void) {
  listeners.add(fn);
  return () => void listeners.delete(fn);
}

/** The form factor right now, without a hook (event handlers, one-off checks). */
export const formFactor = (): FormFactor => current;

/** The layout this window gets; re-renders when it changes (turned device, resized window). */
export function useFormFactor(): FormFactor {
  return useSyncExternalStore(subscribe, formFactor);
}
