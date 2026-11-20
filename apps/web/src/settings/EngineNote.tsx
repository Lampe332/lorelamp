// Why the model does not run, inside the card "Dein Modell" (tab "Modell"): another program holds the
// model server's port (with the way to the port field), or the model stopped after repeated crashes
// (with "Neu starten"), or the supervisor's own note. And the port field itself (tab "Experte").

import { useId, useState, type FormEvent } from "react";
import { ApiError, errorText, postJson, type EngineInfo } from "../api";
import { crashCause, engineNoteText, problemOf, restartModel } from "../app/engineProblem";
import { t } from "../i18n";
import { rich } from "../i18n/rich";
import { openSetting } from "../spot";
import { Icon } from "../ui";

/** Ports the model server may use (no privileged ones); the server checks the same (`bad_port`). */
const PORT_MIN = 1024;
const PORT_MAX = 65535;
const DEFAULT_PORT = 8080;

/** The note under the model's facts; nothing while it runs, is unloaded or switches. */
export function EngineNote({ engine, switching, onChanged }: { engine: EngineInfo; switching: boolean; onChanged: () => Promise<void> }) {
  const [restart, setRestart] = useState<"idle" | "asking" | "started" | "nothing">("idle");
  const [error, setError] = useState("");
  const problem = switching ? null : problemOf(engine);
  const gaveUp = problem?.code === "gave_up";
  // A new stop after a restart starts the button afresh.
  const [wasGaveUp, setWasGaveUp] = useState(gaveUp);
  if (wasGaveUp !== gaveUp) {
    setWasGaveUp(gaveUp);
    if (gaveUp) setRestart("idle");
  }

  if (switching || engine.status === "ready" || engine.unloaded) return null;

  if (problem?.code === "port_busy") {
    return (
      <div className="r2-problem" role="status" data-spot="engine-problem">
        <Icon name="alert" small />
        <div className="r2-problem-body">
          <p>
            <b>{t("models.problem.port", { port: String(problem.port) })}</b> {t("models.problem.port.more")}
          </p>
          <a
            className="btn sm r2-problem-act"
            href="#/settings/expert"
            onClick={(e) => {
              e.preventDefault();
              openSetting("#/settings/expert", "engine-port");
            }}
          >
            {t("models.problem.port.change")}
          </a>
        </div>
      </div>
    );
  }

  if (problem?.code === "gave_up") {
    const cause = crashCause(engine.note);
    const ask = async () => {
      setRestart("asking");
      setError("");
      const r = await restartModel();
      if (typeof r === "string") {
        setError(r);
        setRestart("idle");
      } else setRestart(r.restarting ? "started" : "nothing");
      await onChanged();
    };
    return (
      <div className="r2-problem" role="status" data-spot="engine-problem">
        <Icon name="alert" small />
        <div className="r2-problem-body">
          <p>
            <b>{t("models.problem.crashed", { n: problem.crashes })}</b> {t("models.problem.crashed.more")}
          </p>
          {cause && <p className="r2-problem-cause">{cause}</p>}
          <p className="r2-problem-cause">{rich("models.problem.logs", { logs: <a href="#/settings/logs">{t("models.logs")}</a> })}</p>
          <div className="r2-problem-act">
            <button type="button" className="btn sm" disabled={restart === "asking" || restart === "started"} onClick={() => void ask()}>
              {restart === "asking" || restart === "started" ? t("models.problem.restarting") : t("models.problem.restart")}
            </button>
            {restart === "nothing" && <span className="r2-problem-cause">{t("models.problem.nothing")}</span>}
          </div>
          {error && <p className="bad r2-problem-cause">{error}</p>}
        </div>
      </div>
    );
  }

  return engine.note ? <p className="help">{engineNoteText(engine.note)}</p> : null;
}

/** The managed model server's port (setting `engine_port`): checked here and by the server. */
export function PortField({ engine, onChanged }: { engine: EngineInfo | null; onChanged: () => Promise<void> }) {
  const id = useId();
  const now = engine?.port ?? null;
  const [text, setText] = useState<string | null>(null); // null: shows the port in use
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ bad: boolean; text: string } | null>(null);
  const value = text ?? (now === null ? "" : String(now));
  const n = /^\d{1,5}$/.test(value.trim()) ? Number(value.trim()) : NaN;
  const valid = n >= PORT_MIN && n <= PORT_MAX;
  const busy = engine?.problem?.code === "port_busy" && engine.problem.port === now;

  const save = async (e: FormEvent) => {
    e.preventDefault();
    const range = () => t("models.port.range", { min: String(PORT_MIN), max: String(PORT_MAX) });
    if (!valid) {
      setMessage({ bad: true, text: range() });
      return;
    }
    setSaving(true);
    try {
      await postJson("/settings", { engine_port: n }, "PUT");
      setText(null);
      setMessage({ bad: false, text: t("models.port.saved", { port: String(n) }) });
      await onChanged();
    } catch (err) {
      const why = err instanceof ApiError && err.code === "bad_port" ? range() : t("models.notSaved", { error: errorText(err) });
      setMessage({ bad: true, text: why });
    } finally {
      setSaving(false);
    }
  };

  const shown =
    message ??
    (busy ? { bad: true, text: t("models.port.busy", { port: String(now), other: String(now === DEFAULT_PORT ? 8090 : DEFAULT_PORT) }) } : null);
  return (
    <form className="r2-port" onSubmit={(e) => void save(e)} noValidate>
      <label className="r2-port-label" htmlFor={id}>
        {t("models.port.label")}
      </label>
      <div className="r2-port-row">
        <input
          id={id}
          className={`input r2-port-input num${message?.bad ? " invalid" : ""}`}
          inputMode="numeric"
          autoComplete="off"
          value={value}
          disabled={engine === null}
          aria-invalid={message?.bad || undefined}
          aria-describedby={`${id}-note`}
          data-spot-focus
          onChange={(e) => {
            setText(e.target.value);
            setMessage(null);
          }}
        />
        <button type="submit" className="btn" disabled={saving || engine === null || n === now}>
          {saving ? t("models.saving") : t("models.apply")}
        </button>
      </div>
      <p id={`${id}-note`} className={shown?.bad ? "help bad" : "help"} role={shown ? "status" : undefined}>
        {shown?.text ?? t("models.port.note", { min: String(PORT_MIN), max: String(PORT_MAX) })}
      </p>
    </form>
  );
}
