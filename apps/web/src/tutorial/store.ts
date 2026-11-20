// Where the welcome tour stands. The server keeps it (`GET|PUT /tutorial`, setting "tutorial"), so the
// PC and every phone share it and a reload carries on at the same step; a data folder from before the
// tour counts as done. Whether the tour sits small at the edge while the user does a step elsewhere
// ("docked") belongs to this device only. The state lives outside React: a change of language draws
// the whole app anew (main.tsx) and the tour must stay where it was.

import { useSyncExternalStore } from "react";
import { api, errorText, postJson } from "../api";

export const STEPS = ["welcome", "model", "figure", "persona", "chat"] as const;
export type Step = (typeof STEPS)[number];
export type TourStep = Step | "done";

type State = { step: TourStep | null; docked: boolean };

const DOCK_KEY = "rp-tutorial-docked";

function storedDock(): boolean {
  try {
    return localStorage.getItem(DOCK_KEY) === "1";
  } catch {
    return false;
  }
}

let state: State = { step: null, docked: storedDock() };
const listeners = new Set<() => void>();

function set(next: Partial<State>) {
  state = { ...state, ...next };
  try {
    if (state.docked) localStorage.setItem(DOCK_KEY, "1");
    else localStorage.removeItem(DOCK_KEY);
  } catch {
    // Blocked storage: the tour simply opens in full after a reload.
  }
  for (const l of listeners) l();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};

export const useTour = () => useSyncExternalStore(subscribe, () => state);

const isStep = (v: unknown): v is TourStep => v === "done" || STEPS.includes(v as Step);

let loading: Promise<void> | null = null;

/** Reads the step from the server once per page (a language change draws the app anew, not the page). */
export function loadTour(): Promise<void> {
  loading ??= api<{ step: string }>("/tutorial")
    .then((r) => set({ step: isStep(r.step) ? r.step : "done" }))
    // Without an answer the tour stays away; the app itself is what matters.
    .catch(() => set({ step: "done" }));
  return loading;
}

/** Moves to a step (shown at once, then saved; `done` ends the tour). Answers "" or what went wrong. */
export async function goTo(step: TourStep): Promise<string> {
  set({ step, docked: step === "done" ? false : state.docked });
  try {
    await postJson("/tutorial", { step }, "PUT");
    return "";
  } catch (e) {
    return errorText(e);
  }
}

/** Starts the tour from the beginning (Settings → Allgemein, Ctrl+K). */
export function restartTour(): Promise<string> {
  set({ docked: false });
  return goTo("welcome");
}

/** Puts the tour aside while the user does a step elsewhere in the app, or brings it back. */
export const dockTour = (docked: boolean) => set({ docked });
