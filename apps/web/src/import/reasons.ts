// Why a file (or a link, or a SillyTavern folder) did not come in, in words. Shared by the result
// list, the link import and the SillyTavern import.

import { raw, t, type Key } from "../i18n";

const REASON: Record<string, Key> = {
  unknown_format: "system.reason.unknown_format",
  no_card: "system.reason.no_card",
  no_persona: "system.reason.no_persona",
  broken: "system.reason.broken",
  too_large: "system.reason.too_large",
  bad_name: "system.reason.bad_name",
  bad_upload: "system.reason.bad_upload",
  storage: "system.reason.storage",
  offline: "system.reason.offline",
  not_sillytavern: "system.reason.not_sillytavern",
  bad_user: "system.reason.bad_user",
  not_found: "system.reason.not_found",
  no_character: "system.reason.no_character",
  empty: "system.reason.empty",
  selection_required: "system.reason.selection_required",
};

/** The words for a known code, else null. */
export const knownReason = (code?: string): string | null => (code && REASON[code] ? raw(REASON[code]) : null);

export const reason = (code?: string) => knownReason(code) ?? t("system.reason.unknown");
