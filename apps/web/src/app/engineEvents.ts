// The model's news inside this page: the socket's "engine" and "models" events, passed on to whoever
// shows the model (the status hook, the settings tab "Modell"). A page that starts a switch itself
// sends "switching" here too, so every place shows it before the server's event arrives.

import { useEffect } from "react";
import type { EngineEvent } from "../api";

export type ModelNews = { type: "engine"; event: EngineEvent; profile_id: number | null; port?: number | null } | { type: "models" };

const NAME = "rp-model-news";

export function tellModelNews(news: ModelNews) {
  window.dispatchEvent(new CustomEvent<ModelNews>(NAME, { detail: news }));
}

/** Calls `fn` with each piece of news while the component is mounted. `fn` may change identity. */
export function useModelNews(fn: (news: ModelNews) => void) {
  useEffect(() => {
    const on = (e: Event) => fn((e as CustomEvent<ModelNews>).detail);
    window.addEventListener(NAME, on);
    return () => window.removeEventListener(NAME, on);
  }, [fn]);
}
