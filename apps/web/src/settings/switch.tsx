// The restart of the model server for one profile (switch to it, or new start options applied):
// three steps while it runs, then one line. Shared by the profile rows and the model settings.

import { ctxShort, type EngineInfo } from "../api";
import { t, texts } from "../i18n";
import { Icon } from "../ui";

/** Where a switch to a profile stands. `stop`: the old server goes, `load`: the new one starts. */
export type Phase = "ask" | "stop" | "load" | "done" | "same" | "note" | "bad";
/** `logs`: the text points to the Logs, so a link to them follows it. */
export type Progress = { id: number; phase: Phase; text?: string; logs?: boolean };

const STEPS: { phase: Phase; readonly label: string }[] = [
  texts({ phase: "stop" as Phase }, { label: "models.switch.step.stop" }),
  texts({ phase: "load" as Phase }, { label: "models.switch.step.load" }),
  texts({ phase: "done" as Phase }, { label: "models.switch.step.done" }),
];

export function SwitchProgress({ progress, engine }: { progress: Progress; engine: EngineInfo | null }) {
  const { phase, text } = progress;
  if (phase === "bad" || phase === "note") {
    return (
      <p className={phase === "bad" ? "bad mp-progress" : "help mp-progress"} role="status">
        {text}
        {phase === "bad" && progress.logs && (
          <>
            {" "}
            <a href="#/settings/logs" className="link">
              {t("models.switch.openLogs")}
            </a>
          </>
        )}
      </p>
    );
  }
  if (phase === "same") {
    return (
      <p className="help mp-progress" role="status">
        {t("models.switch.same")}
      </p>
    );
  }
  const at = phase === "ask" ? -1 : STEPS.findIndex((s) => s.phase === phase);
  const ctx = engine?.model.ctx;
  return (
    <div className="mp-progress" role="status" aria-label={t("models.switch.aria")}>
      <ol className="mp-steps">
        {STEPS.map((s, i) => (
          <li key={s.phase} className={i < at || phase === "done" ? "done" : i === at ? "now" : undefined}>
            {i < at || phase === "done" ? <Icon name="check" small /> : <i className={`dot ${i === at ? "loading" : "todo"}`} />}
            {s.label}
          </li>
        ))}
      </ol>
      <span className="mp-step-text">
        {phase === "ask"
          ? t("models.switch.ask")
          : phase === "done"
            ? ctx
              ? t("models.switch.runsCtx", { ctx: ctxShort(ctx) })
              : t("models.switch.runs")
            : phase === "stop"
              ? t("models.switch.stopping")
              : t("models.switch.loading")}
      </span>
    </div>
  );
}
