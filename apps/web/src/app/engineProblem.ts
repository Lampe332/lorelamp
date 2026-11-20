// Why no model runs although one should (`GET /engine` → `problem`), in words: another program holds
// the model server's port, or the supervisor stopped after repeated crashes and waits for "Neu
// starten". The model status in the sidebar and the settings tab "Modell" both say it this way.

import { api, errorText, type EngineInfo, type EngineNote, type EngineProblem } from "../api";
import { t } from "../i18n";
import { tellModelNews } from "./engineEvents";

/** The problem that counts now: none while the model is ready, unloaded or switching. */
export function problemOf(info: EngineInfo | null | undefined): EngineProblem | null {
  if (!info || info.status === "ready" || info.unloaded) return null;
  return info.problem ?? null;
}

/** A word or two for the status pill. */
export function problemLabel(p: EngineProblem): string {
  return p.code === "port_busy" ? t("shell.problem.portBusy") : t("shell.problem.stopped");
}

/** One line under the pill's label. */
export function problemLine(p: EngineProblem): string {
  return p.code === "port_busy" ? t("shell.problem.portLine", { port: String(p.port) }) : t("shell.problem.crashLine", { n: p.crashes });
}

/** The server's note in words, without the hint after a stop for good: the cause ("Modell-Server hat
 *  sich beendet (…).") is what is worth showing next to "Neu starten". Empty without a note. */
export function crashCause(note: EngineNote | null | undefined): string {
  if (!note) return "";
  const detail = note.detail ?? "";
  const port = String(note.port ?? "");
  switch (note.code) {
    case "standalone":
      return t("shell.engineNote.standalone");
    case "no_profile":
      return t("shell.engineNote.no_profile");
    case "config_unreadable":
      return t("shell.engineNote.config_unreadable", { detail });
    case "exited":
      return t("shell.engineNote.exited", { detail });
    case "unqueryable":
      return t("shell.engineNote.unqueryable", { detail });
    case "port_busy":
      return t("shell.engineNote.port_busy", { port });
    case "no_engine_file":
      return t("shell.engineNote.no_engine_file", { detail });
    case "no_model_file":
      return t("shell.engineNote.no_model_file", { detail });
    case "start_failed":
      return t("shell.engineNote.start_failed", { detail });
    case "start_error":
      return t("shell.engineNote.start_error", { detail });
    default:
      return t("shell.engineNote.not_starting");
  }
}

/** The whole note in words, with the hint after a stop for good. */
export function engineNoteText(note: EngineNote | null | undefined): string {
  if (!note) return "";
  const cause = crashCause(note);
  return note.gave_up ? `${cause} ${t("shell.engineNote.gaveUp")}` : cause;
}

/** `POST /engine/restart`: a new series of start attempts. Answers whether something starts, or what went wrong in words. */
export async function restartModel(): Promise<{ restarting: boolean } | string> {
  try {
    const r = await api<{ restarting: boolean }>("/engine/restart", { method: "POST" });
    if (r.restarting) tellModelNews({ type: "engine", event: "waking", profile_id: null });
    return r;
  } catch (e) {
    return t("shell.problem.restartFailed", { error: errorText(e) });
  }
}
