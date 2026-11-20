// Setting up the model in the first steps and the welcome tour. While no engine (llama-server build) is
// installed, one press ("Modell einrichten") picks the build that fits this PC (the PC scan's
// `backend.suggested`) from the newest llama.cpp release, downloads, checks and self-tests it (the same
// way as Einstellungen → Experte → Engines) and then leads on to the model suggestions. The word
// "Engine" stays in the expert part; here it is the program that runs models. A Vulkan build is never
// chosen by itself: the user picks it.

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api, postJson, type Engine } from "../api";
import { useEngineDownload, type EngineDownloadEvent } from "../app/engineDownload";
import { assetLabel, fits, JobView, say, type Asset, type Release, type Scan } from "../settings/PcScan";
import { Icon } from "../ui";
import { t } from "../i18n";
import { EngineWait } from "./extras";

type Pick = { tag: string; asset: Asset };
/** What the press found: one build to start at once, or a choice the user makes (Vulkan, no ROCm runtime). */
type Plan = { start: Pick } | { choose: Pick[]; why: string };

/** The installed engines: undefined while they are on their way, null when they cannot be read (on
 *  another device, for one). */
function useEngines(): [Engine[] | null | undefined, () => void] {
  const [engines, setEngines] = useState<Engine[] | null | undefined>(undefined);
  const load = useCallback(() => {
    api<{ engines: Engine[] }>("/models/engines")
      .then((r) => setEngines(r.engines))
      .catch(() => setEngines(null));
  }, []);
  useEffect(load, [load]);
  return [engines, load];
}

/** The newest release's build for this PC (see the module comment). */
async function plan(): Promise<Plan> {
  // A stored scan first (it is also how tests simulate another PC); without one, scan now.
  let scan = (await api<{ scan: Scan | null }>("/system/scan")).scan;
  if (!scan) scan = (await postJson<{ scan: Scan }>("/system/scan", {})).scan;
  const { releases } = await api<{ releases: Release[] }>("/system/engine-releases");
  const r = releases[0];
  if (!r) throw new Error("no_release");
  const pick = (a: Asset | undefined) => (a ? { tag: r.tag, asset: a } : null);
  const cpu = pick(r.assets.find((a) => a.backend === "cpu"));
  const vulkan = pick(r.assets.find((a) => a.backend === "vulkan"));
  const best = pick(r.assets.find((a) => fits(a, scan)));
  const options = (...p: (Pick | null)[]) => p.filter((x): x is Pick => x !== null);
  if (best && best.asset.backend === "vulkan") {
    return { choose: options(vulkan, cpu), why: t("system.engine.vulkan") };
  }
  if (best && best.asset.backend === "rocm" && !scan?.backend.rocm_runtime_found) {
    return { choose: options(vulkan, cpu), why: t("system.engine.noRocm") };
  }
  const start = best ?? cpu;
  if (!start) throw new Error("no_release");
  return { start };
}

/** `note`: one sentence for when an engine is already there (without one, the box says what it will do
 *  instead, so the step never says two different things). `acts`: the step's own buttons once an engine
 *  is there; without one, "Modell einrichten" here is the only button. */
export function EngineSetup({ note, acts }: { note?: ReactNode; acts: () => ReactNode }) {
  const [engines, reload] = useEngines();
  const [busy, setBusy] = useState(false);
  const [choice, setChoice] = useState<{ choose: Pick[]; why: string } | null>(null);
  const [job, setJob] = useState<EngineDownloadEvent | null>(null);
  const [error, setError] = useState("");

  const onEvent = useCallback(
    (ev: EngineDownloadEvent) => {
      setJob(ev);
      if (ev.phase === "done") reload();
    },
    [reload],
  );
  useEngineDownload(onEvent);

  // Once this box started something it stays (progress, then "weiter"), even when the engine is there.
  const mine = job !== null || choice !== null || busy || error !== "";
  if (!mine && engines === undefined) {
    return (
      <div className="hx-engine" aria-busy="true">
        {note && <p className="hx-engine-why">{note}</p>}
        <EngineWait />
      </div>
    );
  }
  if (!mine && (engines === null || (engines !== undefined && engines.length > 0))) {
    return (
      <div className="hx-engine">
        {note && <p className="hx-engine-why">{note}</p>}
        <div className="hx-step-acts">{acts()}</div>
      </div>
    );
  }

  const download = async (p: Pick) => {
    setBusy(true);
    setError("");
    try {
      setJob((await postJson<{ job: EngineDownloadEvent }>("/models/engines/download", { tag: p.tag, asset: p.asset.name })).job);
      setChoice(null);
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  const go = async () => {
    setBusy(true);
    setError("");
    try {
      const p = await plan();
      if ("start" in p) {
        await download(p.start);
        return;
      }
      setChoice(p);
    } catch (e) {
      setError(e instanceof Error && e.message === "no_release" ? t("system.engine.noRelease") : say(e));
    } finally {
      setBusy(false);
    }
  };

  const running = job !== null && job.phase !== "done" && job.phase !== "failed";
  return (
    <div className="hx-engine" aria-label={t("system.engine.setup")} role="group">
      {job && <JobView job={job} />}
      {job?.phase === "done" ? (
        <div className="hx-step-acts">
          <a className="btn sm primary" href="#/settings/suggestions" data-log-label="Weiter zum Modell">
            {t("system.engine.next")}
            <Icon name="next" small />
          </a>
        </div>
      ) : choice ? (
        <>
          <p className="hx-engine-why">{choice.why}</p>
          <div className="hx-step-acts">
            {choice.choose.map((p, i) => (
              <button key={p.asset.name} type="button" className={`btn sm ${i === 0 ? "primary" : "solid"}`} disabled={busy} onClick={() => void download(p)}>
                {t("system.engine.loadBuild", { build: assetLabel(p.asset) })}
              </button>
            ))}
          </div>
        </>
      ) : (
        !running && (
          <>
            {!job && <p className="hx-engine-why">{t("system.engine.why")}</p>}
            <div className="hx-step-acts">
              <button type="button" className="btn sm primary" disabled={busy} onClick={() => void go()} data-log-label="Engine einrichten">
                <Icon name="download" small />
                {busy ? t("system.engine.searching") : job?.phase === "failed" ? t("system.engine.again") : t("system.engine.setup")}
              </button>
            </div>
          </>
        )
      )}
      {error && (
        <p className="bad" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
