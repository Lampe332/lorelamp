// Unloading the model to free the graphics memory, and loading it again: the sidebar button, the
// quick actions and the settings tab "Modell" all go through here.

import { api, ApiError, errorText } from "../api";
import { t } from "../i18n";
import { tellModelNews } from "./engineEvents";

/** Unloads (`load` false) or loads the model. Answers null, or what went wrong in words. */
export async function setModelLoaded(load: boolean): Promise<string | null> {
  try {
    await api(load ? "/engine/load" : "/engine/unload", { method: "POST" });
    // Every place shows it at once; the server's own event follows.
    tellModelNews({
      type: "engine",
      event: load ? "waking" : "unloaded",
      profile_id: null,
    });
    return null;
  } catch (e) {
    if (e instanceof ApiError && e.code === "busy") return t("models.load.busy");
    return load ? t("models.load.failed", { error: errorText(e) }) : t("models.unload.failed", { error: errorText(e) });
  }
}
