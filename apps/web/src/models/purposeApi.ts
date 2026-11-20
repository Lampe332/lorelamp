// The stored purpose ("Was willst du mit dem Modell machen?") and the settings proposed for it:
// `/api/models/purpose` and `/api/models/profiles/{id}/advice`. One hook keeps the sentence and its
// chips for both places that show them (tab "Vorschläge" and "Beste Einstellung finden").

import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorText, postJson, type Profile, type Sampler } from "../api";
import type { Need, UsePurpose } from "./types";

/** How long typing rests before the sentence is stored and read (ms). */
const REST_MS = 450;

export type Derived = { field: keyof Sampler; value: number; why: "author" | "long_chats" | "group" | "story" | "fast" | "short_replies" | "long_replies" };

/** `GET /models/profiles/{id}/advice`: what "Setzen" would write for the stored purpose. */
export type Advice = {
  purpose: UsePurpose;
  /** Which of the benchmark's recommendations fits the needs. */
  bench_purpose: "speed" | "balanced" | "context";
  /** The measured row taken; null: never measured. */
  measured: { row: number; purpose: "speed" | "balanced" | "context"; ctx: number; kv: string; n_cpu_moe: number; tg: number | null; pp: number | null; uncertain: boolean; at: number | null } | null;
  load_args: string[] | null;
  sampler: Sampler;
  derived: Derived[];
  author_source: string | null;
  /** The profile already has every proposed value. */
  same: boolean;
};

export const getAdvice = (profileId: number) => api<Advice>(`/models/profiles/${profileId}/advice`);
export const applyAdvice = (profileId: number) => postJson<{ profile: Profile; engine: "restarting" | "saved"; advice: Advice }>(`/models/profiles/${profileId}/advice/apply`, {});

/** The purpose with its field state: `text` is what the field shows, `p` what the server last
 *  answered. Typing stores the sentence after a short rest (the needs are read from it again);
 *  `toggle` stores the needs chosen by hand. `onStored` runs after every stored change. */
export function usePurpose(onStored?: () => void) {
  const [p, setP] = useState<UsePurpose | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState("");
  const timer = useRef<number | undefined>(undefined);
  // The newest request wins: a slow answer for an older text never replaces a newer one.
  const asked = useRef(0);
  const stored = useRef(onStored);
  useEffect(() => {
    stored.current = onStored;
  });

  const load = useCallback(async () => {
    const n = ++asked.current;
    try {
      const got = await api<UsePurpose>("/models/purpose");
      if (n !== asked.current) return;
      setP(got);
      setText(got.text);
    } catch (e) {
      if (n === asked.current) setError(errorText(e));
    }
  }, []);
  useEffect(() => {
    void load();
    return () => window.clearTimeout(timer.current);
  }, [load]);

  const store = useCallback(async (body: { text: string; needs?: Need[] }) => {
    const n = ++asked.current;
    try {
      const got = await postJson<UsePurpose>("/models/purpose", body, "PUT");
      if (n !== asked.current) return;
      setP(got);
      setError("");
      stored.current?.();
    } catch (e) {
      if (n === asked.current) setError(errorText(e));
    }
  }, []);

  const type = (next: string) => {
    setText(next);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void store({ text: next }), REST_MS);
  };
  const toggle = (need: Need) => {
    if (!p) return;
    window.clearTimeout(timer.current);
    const needs = p.needs.includes(need) ? p.needs.filter((n) => n !== need) : [...p.needs, need];
    // Shown at once; the server's answer (chip order, hits) follows.
    setP({ ...p, needs });
    void store({ text, needs });
  };
  const clear = () => {
    window.clearTimeout(timer.current);
    setText("");
    void store({ text: "" });
  };
  return { p, text, error, type, toggle, clear };
}
