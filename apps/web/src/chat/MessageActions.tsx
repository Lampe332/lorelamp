// The row of actions under a message: copy, edit, rewind, branch off, and on the chat's last reply
// "Neu generieren" and "Weiter schreiben" in front of them.
//
// Adding an action: one more `.icon-btn` with an `aria-label` and a `title`, either here after
// "Ab hier abzweigen" or handed in as `children` by the message (they are drawn last). The row
// wraps on a phone, so a further button needs no other change. `disabled={locked}` for anything
// that must wait while the model writes in this chat.

import type { ReactNode } from "react";
import type { Msg } from "../api";
import { t } from "../i18n";
import { Icon } from "../ui";

/** What can be done with a message. One stable object, so a message re-renders only when it changes. */
export type Actions = {
  copy: (m: Msg) => void;
  edit: (m: Msg) => void;
  editText: (id: number, text: string) => void;
  saveEdit: () => void;
  cancelEdit: () => void;
  rewind: (m: Msg) => void;
  fork: (id: number) => void;
  /** Another variant of the chat's last reply. */
  regen: (m: Msg) => void;
  /** More text at the end of the chat's last reply. */
  more: (m: Msg) => void;
};

/** Whether this message can be written again or on: not this one (only the chat's last reply to a
 *  message of yours), yes, or not right now (the model writes somewhere, or cannot be reached). */
export type Write = "no" | "ready" | "wait";

type Props = {
  m: Msg;
  /** The model writes in this chat: the story must not change under it. */
  locked: boolean;
  /** The greeting stays: a chat without it would not be this card's chat. */
  canRewind: boolean;
  copied: boolean;
  write: Write;
  act: Actions;
  /** Further actions of the message, drawn after the built-in ones. */
  children?: ReactNode;
};

export function MessageActions({ m, locked, canRewind, copied, write, act, children }: Props) {
  return (
    <div className="msg-actions">
      {write !== "no" && (
        <span className="cv-write">
          {/* `x7-regen`: on a tablet a swipe past the last variant clicks this (tablet/swipe.ts). */}
          <button
            className="icon-btn x7-regen"
            title={t("chat.act.regen.title")}
            aria-label={t("chat.act.regen")}
            disabled={locked || write === "wait"}
            onClick={() => act.regen(m)}
          >
            <Icon name="refresh" small />
          </button>
          <button
            className="icon-btn"
            title={t("chat.act.more.title")}
            aria-label={t("chat.act.more")}
            disabled={locked || write === "wait"}
            onClick={() => act.more(m)}
          >
            <Icon name="goon" small />
          </button>
        </span>
      )}
      <button className="icon-btn" title={copied ? t("chat.act.copied") : t("chat.act.copy")} aria-label={t("chat.act.copy")} onClick={() => act.copy(m)}>
        <Icon name={copied ? "check" : "copy"} small />
      </button>
      <button className="icon-btn" title={t("chat.act.edit")} aria-label={t("chat.edit.label")} disabled={locked} onClick={() => act.edit(m)}>
        <Icon name="pencil" small />
      </button>
      {canRewind && (
        <button className="icon-btn" title={t("chat.act.rewind.title")} aria-label={t("chat.act.rewind")} disabled={locked} onClick={() => act.rewind(m)}>
          <Icon name="rewind" small />
        </button>
      )}
      <button className="icon-btn" title={t("chat.act.fork.title")} aria-label={t("chat.act.fork")} disabled={locked} onClick={() => act.fork(m.id)}>
        <Icon name="fork" small />
      </button>
      {children}
    </div>
  );
}
