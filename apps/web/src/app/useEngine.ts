// The model server's state, polled: fast while it is not ready (or switching), slow once it is (or
// unloaded), and at once when the socket says the model or the profiles changed.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type EngineInfo, type EngineNote, type EngineStatus } from "../api";
import { logEvent } from "../log";
import { useModelNews, type ModelNews } from "./engineEvents";
import { engineNoteText } from "./engineProblem";

/** A switch that never reports back (socket gone, server restarted) is forgotten after this. */
const SWITCH_MAX_MS = 180_000;

export function useEngine() {
  const [engine, setEngine] = useState<EngineStatus>("offline");
  // The server's note (a code), put in words at each render so it follows the app's language.
  const [note, setNote] = useState<EngineNote | null>(null);
  const [info, setInfo] = useState<EngineInfo | null>(null);
  const [switching, setSwitching] = useState(false);
  const [switchTo, setSwitchTo] = useState<number | null>(null); // the profile a switch goes to
  // An unloaded model is on its way up again (a message or "Laden"), until it is ready or gives up.
  const [waking, setWaking] = useState(false);
  const pollNow = useRef<() => void>(() => {});
  // While switching: has the old server been seen gone? Then the next "ready" poll ends the switch
  // even when the socket missed the "ready" event.
  const sw = useRef({ on: false, since: 0, sawDown: false });

  const endSwitch = useCallback(() => {
    sw.current = { on: false, since: 0, sawDown: false };
    setSwitching(false);
    setSwitchTo(null);
  }, []);

  useEffect(() => {
    let timer: number | undefined;
    let cancelled = false;
    let seen: { status: EngineStatus | ""; note: string } = { status: "", note: "" };
    let gen = 0; // a poll asked for while one runs makes the running one's answer stale
    const poll = async () => {
      window.clearTimeout(timer);
      const mine = ++gen;
      let next: EngineStatus = "offline";
      let nextNote: EngineNote | null = null;
      let got: EngineInfo | null = null;
      try {
        got = await api<EngineInfo>("/engine");
        next = got.status;
        nextNote = got.note ?? null;
      } catch {
        next = "offline";
      }
      if (cancelled || mine !== gen) return;
      // The log keeps the code and its detail (a record for the owner, not the UI's words).
      const note = nextNote ? [nextNote.code, nextNote.detail, nextNote.port, nextNote.gave_up ? "gave_up" : ""].filter(Boolean).join(" ") : "";
      if (next !== seen.status) {
        const level = next === "error" ? "error" : seen.status === "ready" ? "warn" : "info";
        logEvent(level, "model", `Modell-Status: ${seen.status || "–"} → ${next}`, { data: { status: next, note: note || undefined } });
      } else if (note !== seen.note) {
        logEvent("info", "model", note ? "Modell-Hinweis geändert" : "Modell-Hinweis entfällt", { data: { status: next, note: note || undefined } });
      }
      seen = { status: next, note };
      const s = sw.current;
      if (s.on) {
        if (next !== "ready") s.sawDown = true;
        else if (s.sawDown || Date.now() - s.since > SWITCH_MAX_MS) endSwitch();
      }
      setEngine(next);
      setNote(nextNote);
      if (got) setInfo(got);
      if (next === "ready" || got?.unloaded) setWaking(false);
      timer = window.setTimeout(poll, (next === "ready" || got?.unloaded) && !sw.current.on ? 10000 : 1500);
    };
    pollNow.current = () => void poll();
    void poll();
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [endSwitch]);

  const onNews = useCallback(
    (n: ModelNews) => {
      if (n.type === "engine") {
        if (n.event === "switching") {
          if (!sw.current.on) sw.current = { on: true, since: Date.now(), sawDown: false };
          setSwitching(true);
          setSwitchTo(n.profile_id);
        } else if (n.event === "ready" || n.event === "failed" || n.event === "crashed" || n.event === "port_busy" || n.event === "foreign") {
          // Nothing of ours starts on a held port (port_busy), or another llama-server answers there (foreign).
          endSwitch();
        }
        if (n.event === "waking") setWaking(true);
        else if (n.event === "ready" || n.event === "failed" || n.event === "unloaded" || n.event === "port_busy") setWaking(false);
      }
      pollNow.current();
    },
    [endSwitch],
  );
  useModelNews(onNews);

  return { engine, engineNote: engineNoteText(note), info, switching, switchTo, waking, unloaded: info?.unloaded ?? false };
}
