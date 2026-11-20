// The start screen's own calls to the server.

import { errorText, postJson } from "../api";
import { t } from "../i18n";

/** Sets or removes a figure's star (the same star as in the library). Answers null, or what went wrong in words. */
export async function setFavorite(characterId: number, favorite: boolean): Promise<string | null> {
  try {
    await postJson(`/characters/${characterId}/favorite`, { favorite }, "PUT");
    return null;
  } catch (e) {
    return t("system.hub.fav.failed", { error: errorText(e) });
  }
}
