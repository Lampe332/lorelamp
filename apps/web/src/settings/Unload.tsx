// The settings tab "Modell", inside the card "Dein Modell": giving the graphics memory back. Unload
// the model now, or after a quiet time (off by default); the next message loads it again by itself.

import { useId, useState } from "react";
import { errorText, postJson, type EngineInfo } from "../api";
import { setModelLoaded } from "../app/engineLoad";
import { problemOf } from "../app/engineProblem";
import { t } from "../i18n";

const CHOICES = [0, 5, 15, 30, 60];
const choiceName = (min: number) => (min === 0 ? t("models.idle.never") : t("models.idle.minutes", { n: min }));

/** Only an app that starts the model itself can unload it; for any other the line stays away. */
export function MemoryLine({ info, onChanged }: { info: EngineInfo; onChanged: () => Promise<void> }) {
  const [problem, setProblem] = useState("");
  const [saving, setSaving] = useState(false);
  const id = useId();
  if (!info.managed) return null;

  const choose = async (minutes: number) => {
    setSaving(true);
    try {
      await postJson("/engine/idle", { minutes }, "PUT");
      setProblem("");
      await onChanged();
    } catch (e) {
      setProblem(t("models.notSaved", { error: errorText(e) }));
    } finally {
      setSaving(false);
    }
  };
  // A held port or a stop after crashes: nothing is loading (the card above says why).
  const stopped = problemOf(info) !== null;
  const toggle = async () => {
    setProblem((await setModelLoaded(info.unloaded)) ?? "");
    await onChanged();
  };

  return (
    <div className="mx-mem" data-spot="idle-unload">
      <div className="mx-mem-now">
        <span className="unload-state" role="status">
          <i className={`dot ${info.unloaded ? "idle" : info.status === "ready" ? "ready" : stopped ? "offline" : "loading"}`} />
          {info.unloaded ? t("models.mem.unloaded") : info.status === "ready" ? t("models.mem.loaded") : stopped ? t("models.mem.notLoaded") : t("models.mem.loading")}
        </span>
        {!stopped && (
          <button type="button" className="btn" onClick={() => void toggle()} title={info.unloaded ? undefined : t("models.mem.unload.title")}>
            {info.unloaded ? t("models.mem.load") : t("models.mem.unload")}
          </button>
        )}
      </div>
      <div className="mx-mem-idle">
        <label htmlFor={id}>{t("models.idle.label")}</label>
        <select id={id} className="input" value={info.idle_unload_min} disabled={saving} onChange={(e) => void choose(Number(e.target.value))} data-spot-focus>
          {CHOICES.map((min) => (
            <option key={min} value={min}>
              {choiceName(min)}
            </option>
          ))}
        </select>
        <span className="mx-mem-note">{t("models.idle.note")}</span>
      </div>
      {problem && <p className="bad">{problem}</p>}
    </div>
  );
}
