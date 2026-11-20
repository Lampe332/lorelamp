// Step 3 of the card builder (and the side column on wide screens): the card as the library shows it,
// the first message as the chat shows it, and how much of the prompt the card takes.

import { useState } from "react";
import { Prose } from "../chat/Prose";
import { t } from "../i18n";
import type { Picked } from "../image";
import { hueStyle, Icon, Portrait } from "../ui";
import { resolve, sizes, type Draft } from "./draft";

type Props = {
  draft: Draft;
  picked: Picked | null;
  pos: [number, number];
  /** The name `{{user}}` becomes: the active persona's. */
  userName: string;
  /** The full view (its own step) or the narrow side column. */
  full: boolean;
};

/** The short text under the card, as the library makes it: the first line of the notes, the
 *  personality or the scenario. */
const blurbOf = (d: Draft) => [d.creator_notes, d.personality, d.scenario].map((s) => s.trim().split("\n")[0] ?? "").find(Boolean) ?? "";

export function Preview({ draft, picked, pos, userName, full }: Props) {
  const name = draft.name.trim() || t("builder.pv.noName");
  const who = { id: 0, name, has_avatar: !!picked, avatar_rev: 0, avatar_pos: pos };
  const greetings = [draft.first_mes, ...draft.alternate_greetings].filter((g) => g.trim());
  const [at, setAt] = useState(0);
  const shown = greetings[Math.min(at, greetings.length - 1)] ?? "";
  const size = sizes(draft);
  const missing = [
    !draft.name.trim() && t("builder.pv.miss.name"),
    !draft.description.trim() && t("builder.pv.miss.description"),
    !draft.first_mes.trim() && t("builder.pv.miss.first"),
  ].filter((m): m is string => !!m);
  const tags = draft.tags.map((x) => x.trim()).filter(Boolean);

  return (
    <div className={`cb-pv${full ? " full" : ""}`} aria-label={t("builder.pv.title")} role="region">
      <div className="cb-pv-card">
        <article className="cc" style={hueStyle(name)}>
          <div className="cc-open cb-pv-open">
            <Portrait who={who} className="art" preview={picked?.url} pos={pos} full />
            <span className="body">
              <b>{name}</b>
              <span className="s">{blurbOf(draft) || t("builder.pv.noBlurb")}</span>
              <span className="meta">{tags.length ? tags.slice(0, 3).join(", ") : t("builder.pv.noTags")}</span>
            </span>
          </div>
        </article>
      </div>

      <div className="cb-pv-rest">
      <div className="cb-pv-chat">
        <div className="cb-pv-head">
          <span className="cb-pv-label">{t("builder.pv.chat")}</span>
          {greetings.length > 1 && (
            <span className="cb-pv-turn">
              <button type="button" className="icon-btn" aria-label={t("builder.pv.prev")} disabled={at === 0} onClick={() => setAt((n) => Math.max(0, n - 1))}>
                <Icon name="back" small />
              </button>
              <span className="num">{t("builder.pv.greetingOf", { n: Math.min(at, greetings.length - 1) + 1, total: greetings.length })}</span>
              <button type="button" className="icon-btn cb-flip" aria-label={t("builder.pv.next")} disabled={at >= greetings.length - 1} onClick={() => setAt((n) => Math.min(greetings.length - 1, n + 1))}>
                <Icon name="back" small />
              </button>
            </span>
          )}
        </div>
        {shown ? (
          <article className="msg ai cb-pv-msg" style={hueStyle(name)}>
            <header className="msg-head">
              <Portrait who={who} className="av" preview={picked?.url} pos={pos} />
              <b>{name}</b>
            </header>
            <Prose text={resolve(shown, name, userName)} streaming={false} />
          </article>
        ) : (
          <p className="help cb-pv-none">{t("builder.pv.noGreeting")}</p>
        )}
      </div>

      <dl className="cb-pv-size">
        <div>
          <dt>{t("builder.pv.size.always")}</dt>
          <dd className="num">{t("builder.tokens", { n: size.always })}</dd>
        </div>
        <div>
          <dt>{t("builder.pv.size.greeting")}</dt>
          <dd className="num">{t("builder.tokens", { n: size.greeting })}</dd>
        </div>
        {size.lore > 0 && (
          <div>
            <dt>{t("builder.pv.size.lore")}</dt>
            <dd className="num">{t("builder.tokens", { n: size.lore })}</dd>
          </div>
        )}
        {size.perTurn > 0 && (
          <div>
            <dt>{t("builder.pv.size.turn")}</dt>
            <dd className="num">{t("builder.tokens", { n: size.perTurn })}</dd>
          </div>
        )}
      </dl>
      {full && <p className="help">{t("builder.pv.size.help")}</p>}

      {missing.length > 0 && (
        <ul className="cb-pv-miss">
          {missing.map((m) => (
            <li key={m}>{m}</li>
          ))}
        </ul>
      )}
      </div>
    </div>
  );
}
