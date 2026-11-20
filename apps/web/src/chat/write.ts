// The chat's last reply once more: another variant of it ("Neu generieren") or more text at its
// end ("Weiter schreiben"). The server refuses both while the model writes (409 `busy`); the text
// itself arrives over the socket like any reply, and the chat's history follows it.

import { ApiError, ERROR_TEXT, postJson } from "../api";
import { keysUnder, t } from "../i18n";

export const regenerate = (messageId: number) => postJson(`/messages/${messageId}/regenerate`, {});
export const continueReply = (messageId: number) => postJson(`/messages/${messageId}/continue`, {});

/** The refusals said in this area's own words (`chat.write.<code>`); every other code in the shell's. */
const REFUSED = keysUnder("chat.write.");

/** Why the server did not start writing, in words. */
export function writeError(e: unknown): string {
  if (!(e instanceof ApiError)) return ERROR_TEXT.unknown;
  const own = REFUSED.find((k) => k === `chat.write.${e.code}`);
  return own ? t(own) : (ERROR_TEXT[e.code] ?? ERROR_TEXT.unknown);
}
