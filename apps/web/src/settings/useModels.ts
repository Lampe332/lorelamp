// What the settings tabs "Modell" and "Experte" both read: the running model, the profiles, the
// engines and the model files, plus the switch to another profile or model with its progress (which
// comes over the socket as "engine" events).

import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, api, errorText, postJson, type ActivateAnswer, type Engine, type EngineInfo, type LocalModel, type LocalModels, type Profile } from "../api";
import { tellModelNews, useModelNews, type ModelNews } from "../app/engineEvents";
import { t } from "../i18n";
import { respot } from "../spot";
import type { Progress } from "./switch";

/** What the server answered to "activate", for the ones that do not restart anything. */
type Noted = Exclude<ActivateAnswer, "restarting" | "unchanged">;
const ACTIVATE_NOTE = (a: Noted) => t(`models.activate.${a}`);

export function useModels() {
  const [engine, setEngine] = useState<EngineInfo | null>(null);
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [engines, setEngines] = useState<Engine[]>([]);
  const [local, setLocal] = useState<LocalModels | null>(null);
  const [localError, setLocalError] = useState("");
  const [error, setError] = useState("");
  const [progress, setProgress] = useState<Progress | null>(null);
  /** The model file a "Benutzen" runs for (its row shows the switch). */
  const [usePath, setUsePath] = useState<string | null>(null);
  const progressNow = useRef(progress); // for the activation, which outlives a render
  useEffect(() => {
    progressNow.current = progress;
  });

  const loadEngine = useCallback(async () => {
    try {
      setEngine(await api<EngineInfo>("/engine"));
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  const loadProfiles = useCallback(async () => {
    try {
      const [p, e] = await Promise.all([api<{ profiles: Profile[] }>("/models/profiles"), api<{ engines: Engine[] }>("/models/engines")]);
      setProfiles(p.profiles);
      setEngines(e.engines);
      setError("");
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  const loadLocal = useCallback(async () => {
    try {
      setLocal(await api<LocalModels>("/models/local"));
      setLocalError("");
    } catch (e) {
      setLocalError(errorText(e));
    }
  }, []);
  const loadAll = useCallback(async () => {
    await Promise.all([loadProfiles(), loadLocal()]);
  }, [loadProfiles, loadLocal]);

  useEffect(() => {
    void loadEngine();
    void loadProfiles();
    void loadLocal();
  }, [loadEngine, loadProfiles, loadLocal]);

  // A place asked for with Ctrl+K before the profiles were there is shown once they are.
  const arrived = profiles !== null && engine !== null;
  useEffect(() => {
    if (arrived) respot();
  }, [arrived]);

  // While the server is not ready, ask again now and then (the socket's "ready" ends it sooner).
  useEffect(() => {
    if (!engine || engine.status === "ready") return;
    const t = window.setTimeout(() => void loadEngine(), 1500);
    return () => window.clearTimeout(t);
  }, [engine, loadEngine]);

  // A finished switch says so for a moment, then the row is quiet again.
  useEffect(() => {
    if (progress?.phase !== "done" && progress?.phase !== "same") return;
    const t = window.setTimeout(() => setProgress((p) => (p === progress ? null : p)), 6000);
    return () => window.clearTimeout(t);
  }, [progress]);
  useEffect(() => {
    if (!progress) setUsePath(null);
  }, [progress]);

  const onNews = useCallback(
    (n: ModelNews) => {
      if (n.type === "models") {
        void loadAll();
        return;
      }
      void loadEngine();
      const id = n.profile_id;
      setProgress((p) => {
        if (n.event === "switching") return { id: id ?? p?.id ?? 0, phase: "stop" };
        if (!p || (id !== null && id !== p.id) || p.phase === "done" || p.phase === "bad" || p.phase === "note") return p;
        if (n.event === "starting") return { ...p, phase: "load" };
        if (n.event === "ready") return { ...p, phase: "done" };
        if (n.event === "port_busy")
          return { ...p, phase: "bad", text: n.port ? t("models.switch.portBusy", { port: String(n.port) }) : t("models.switch.modelPortBusy") };
        if (n.event === "foreign") return { ...p, phase: "note", text: ACTIVATE_NOTE("foreign") };
        if (n.event === "crashed" || n.event === "failed") return { ...p, phase: "bad", text: t("models.switch.failed"), logs: true };
        return p;
      });
    },
    [loadEngine, loadAll],
  );
  useModelNews(onNews);

  /** What the server answered to activating `id`, as progress. */
  const settle = async (id: number, engineState: ActivateAnswer) => {
    if (engineState === "restarting") {
      // The sidebar shows it at once, unless the server's own events were faster than this answer.
      if (progressNow.current?.phase === "ask") {
        setProgress({ id, phase: "ask" });
        tellModelNews({ type: "engine", event: "switching", profile_id: id });
      }
    } else if (engineState === "unchanged") {
      setProgress({ id, phase: "same" });
    } else {
      setProgress({ id, phase: "note", text: ACTIVATE_NOTE(engineState) });
    }
    await Promise.all([loadAll(), loadEngine()]);
  };
  const failed = (id: number, e: unknown) => {
    const text = e instanceof ApiError && e.code === "busy" ? t("models.switch.busy") : t("models.use.failed", { error: errorText(e) });
    setProgress({ id, phase: "bad", text });
  };

  const activate = async (p: Profile) => {
    setUsePath(null);
    setProgress({ id: p.id, phase: "ask" });
    try {
      const r = await api<{ profile: Profile; engine: ActivateAnswer }>(`/models/profiles/${p.id}/activate`, { method: "POST" });
      await settle(p.id, r.engine);
    } catch (e) {
      failed(p.id, e);
    }
  };

  /** "Benutzen": the model's profile (made now if it has none) becomes the active one. */
  const use = async (m: LocalModel) => {
    setUsePath(m.path);
    setProgress({ id: -1, phase: "ask" });
    try {
      const r = await postJson<{ profile: Profile; created: boolean; engine: ActivateAnswer }>("/models/local/use", { path: m.path });
      await settle(r.profile.id, r.engine);
    } catch (e) {
      failed(-1, e);
    }
  };

  const active = profiles?.find((p) => p.active) ?? null;
  const switching = progress !== null && (progress.phase === "ask" || progress.phase === "stop" || progress.phase === "load");
  return { engine, profiles, engines, local, localError, error, progress, usePath, active, switching, loadEngine, loadProfiles, loadAll, activate, use };
}

export type Models = ReturnType<typeof useModels>;
