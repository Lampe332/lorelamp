// "Du in dieser Geschichte" (character settings): what the card says about the user (`{{user}}`),
// compared with one persona, and what applies in the chat when the two differ. The comparison is
// the server's local text check (`GET /characters/{id}/user-check`), sorted into topics in
// `userfit/topics.ts`; the choice of what applies is saved at once (`PUT /characters/{id}/user-priority`,
// null = as the persona says).

import { Fragment, useEffect, useState, type ReactNode } from "react";
import { api, ApiError, errorText, postJson, type CharInfo, type CheckItem, type Fit, type Persona, type Priority, type UserCheck } from "./api";
import { locale, t } from "./i18n";
import { rich } from "./i18n/rich";
import { logEvent } from "./log";
import { PRIORITY } from "./persona/common";
import { openSetting } from "./spot";
import { Icon, Portrait, type IconName } from "./ui";
import { sortItems, type Match, type Place, type Topic } from "./userfit/topics";

const STATE: Record<Fit, { title: () => string; icon: IconName }> = {
  ok: { title: () => t("cards.fit.state.ok"), icon: "check" },
  overlap: { title: () => t("cards.fit.state.overlap"), icon: "overlap" },
  conflict: { title: () => t("cards.fit.state.conflict"), icon: "alert" },
};

const MATCH: Record<Match, () => string> = {
  card: () => t("cards.fit.match.card"),
  ok: () => t("cards.fit.match.ok"),
  adds: () => t("cards.fit.match.adds"),
  overlap: () => t("cards.fit.match.overlap"),
  conflict: () => t("cards.fit.match.conflict"),
};

const priorityLabel = (p: Priority) => PRIORITY.find((x) => x.value === p)?.label ?? p;

/** "A, B und C" as the app's language joins a list. */
const joined = (parts: string[]) => new Intl.ListFormat(locale(), { type: "conjunction" }).format(parts);

/** A card sentence with `{{user}}` set apart, so one sees at once where it speaks about you. */
function Excerpt({ text }: { text: string }) {
  return (
    <>
      {text.split(/(\{\{user\}\})/i).map((part, i) =>
        /^\{\{user\}\}$/i.test(part) ? (
          <span className="uf-u" key={i}>
            {"{{user}}"}
          </span>
        ) : (
          <Fragment key={i}>{part}</Fragment>
        ),
      )}
    </>
  );
}

/** "dein Aussehen und deine Persönlichkeit" from the kinds that overlap. */
function overlapWords(items: CheckItem[]): string {
  const words = [...new Set(items.filter((i) => i.verdict === "overlap").map((i) => i.kind))].map((k) =>
    k === "appearance" ? t("cards.fit.both.appearance") : k === "personality" ? t("cards.fit.both.personality") : k,
  );
  return words.length > 0 ? joined(words) : t("cards.fit.both.you");
}

/** "als männlich" / "mit dem Namen „Kai“" for each contradiction. */
function conflictWords(items: CheckItem[]): string {
  const seen = new Set<string>();
  const parts: string[] = [];
  for (const i of items) {
    if (i.verdict !== "conflict") continue;
    const part =
      i.kind === "gender"
        ? i.value === "male"
          ? t("cards.fit.how.male")
          : i.value === "female"
            ? t("cards.fit.how.female")
            : t("cards.fit.how.gender", { gender: i.value ?? "" })
        : i.kind === "name"
          ? t("cards.fit.how.name", { name: i.value ?? "" })
          : "";
    if (part && !seen.has(part)) {
      seen.add(part);
      parts.push(part);
    }
  }
  return parts.length > 0 ? joined(parts) : "";
}

/** The places of the card, folded: "Belege (3)". */
function Places({ list }: { list: Place[] }) {
  return (
    <details className="uf-proof">
      <summary data-log-label="Belege auf-/zuklappen">
        <Icon name="chevron" small />
        {t("cards.fit.proof", { n: list.length })}
      </summary>
      <ul>
        {list.map((p, i) => (
          <li key={i}>
            <span className="uf-field">{p.fields.join(" · ")}</span>
            <p>
              <Excerpt text={p.excerpt} />
            </p>
          </li>
        ))}
      </ul>
    </details>
  );
}

function TopicRow({ topic, persona }: { topic: Topic; persona: Persona | null }) {
  const differs = (topic.match === "conflict" || topic.match === "overlap") && topic.personaText !== undefined && persona;
  return (
    <li className={`uf-topic is-${topic.match}`}>
      <div className="uf-topic-head">
        <h3>{topic.label}</h3>
        <span className={`uf-tag is-${topic.match}`}>{MATCH[topic.match]()}</span>
      </div>
      <div className="uf-topic-body">
        {topic.values.length > 0 ? (
          <ul className="uf-vals" aria-label={t("cards.fit.values", { topic: topic.label })}>
            {topic.values.map((v) => (
              <li key={v}>{v}</li>
            ))}
          </ul>
        ) : (
          <p className="uf-none">{topic.kind === "role" ? t("cards.fit.none.role") : t("cards.fit.none.trait")}</p>
        )}
        {differs && (
          <a className="uf-theirs" href={`#/personas/${persona.id}`} title={t("cards.fit.theirs.title", { name: persona.name })}>
            <Portrait who={persona} kind="persona" className="av" />
            <span>
              <b>{t("cards.fit.theirs", { name: persona.name })}</b> {topic.kind === "name" ? t("cards.quoted", { text: topic.personaText ?? "" }) : topic.personaText}
            </span>
          </a>
        )}
        <Places list={topic.places} />
      </div>
    </li>
  );
}

type Props = {
  /** The character as the library has it (live: other devices' changes show). */
  character: CharInfo;
  personas: Persona[];
  activeId: number | null;
  /** Changes whenever the library changed; the check is fetched again then. */
  rev: unknown;
};

export function UserFit({ character, personas, activeId, rev }: Props) {
  const id = character.id;
  // null: the persona new chats start with (the server's choice); else one picked here to compare.
  const [pick, setPick] = useState<number | null>(null);
  const [check, setCheck] = useState<UserCheck | null>(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState<{ p: Priority | null } | null>(null);
  const [state, setState] = useState<"idle" | "saving" | "saved">("idle");
  const [saveError, setSaveError] = useState("");

  useEffect(() => {
    let stale = false;
    api<UserCheck>(`/characters/${id}/user-check${pick !== null ? `?persona_id=${pick}` : ""}`)
      .then((c) => {
        if (stale) return;
        setCheck(c);
        setError("");
      })
      .catch((e) => {
        if (stale) return;
        // The persona picked here was deleted meanwhile: compare with the one of new chats again.
        if (e instanceof ApiError && e.code === "persona_missing") setPick(null);
        else setError(errorText(e));
      });
    return () => {
      stale = true;
    };
  }, [id, pick, rev]);

  // The library has caught up with the choice.
  const saved = character.user_priority;
  useEffect(() => {
    if (pending && pending.p === saved) setPending(null);
  }, [pending, saved]);
  const chosen = pending ? pending.p : saved;

  const choose = async (p: Priority | null) => {
    if (p === chosen) return;
    logEvent("info", "settings", "Vorrang der Figur gesetzt", { character_id: id, data: { priority: p } });
    setPending({ p });
    setState("saving");
    setSaveError("");
    try {
      await postJson(`/characters/${id}/user-priority`, { priority: p }, "PUT");
      setState("saved");
    } catch (e) {
      setPending(null);
      setSaveError(errorText(e));
      setState("idle");
    }
  };

  const newChatPersona = personas.find((p) => p.id === character.default_persona_id) ?? personas.find((p) => p.id === activeId) ?? null;
  const compared = check ? (personas.find((p) => p.id === check.persona_id) ?? null) : null;
  const effective: Priority = chosen ?? compared?.card_priority ?? "persona";

  const label = (
    <div className="set-label">
      <h2>{t("cards.fit.title")}</h2>
      <p>{t("cards.fit.text")}</p>
    </div>
  );

  if (!check) {
    return (
      <section className="set-row" data-spot="user-fit">
        {label}
        <div className="set-panel" aria-busy={!error}>
          {error ? <p className="help bad">{t("cards.fit.loadFailed", { error })}</p> : <span className="bar-ph" />}
        </div>
      </section>
    );
  }

  const items = check.items;
  const status = check.status;
  const { topics, rules } = sortItems(items, compared, character.name);

  // The core line: name, roles and gender the card gives you, in that order.
  const core = [
    ...(topics.find((tp) => tp.kind === "name")?.values.map((v) => t("cards.quoted", { text: v })) ?? []),
    ...(topics.find((tp) => tp.kind === "role")?.values ?? []),
    ...(topics.find((tp) => tp.kind === "gender")?.values ?? []),
  ];
  const spots = topics.reduce((n, tp) => n + tp.places.length, 0);
  let headline: ReactNode;
  if (core.length > 0)
    headline = rich("cards.fit.core", {
      list: core.map((v, i) => (
        <Fragment key={i}>
          {i > 0 && ", "}
          <b>{v}</b>
        </Fragment>
      )),
    });
  else if (items.length === 0) headline = t("cards.fit.nothing");
  else if (spots === 0) headline = t("cards.fit.rulesOnly");
  else headline = t("cards.fit.spots", { n: spots });

  const effect = effective === "card" ? t("cards.fit.effect.card") : t("cards.fit.effect.persona");
  // Each sentence names the compared persona, or says "your persona" when there is none to name.
  const name = compared?.name;
  let sentence: string;
  if (status === "conflict") {
    const how = conflictWords(items) || t("cards.fit.how.other");
    sentence = name ? t("cards.fit.conflict", { how, name, effect }) : t("cards.fit.conflict.you", { how, effect });
  } else if (status === "overlap") {
    const what = overlapWords(items);
    sentence = name ? t("cards.fit.overlap", { what, name, effect }) : t("cards.fit.overlap.you", { what, effect });
  } else if (items.length === 0) sentence = name ? t("cards.fit.empty", { name }) : t("cards.fit.empty.you");
  else sentence = name ? t("cards.fit.fits", { name }) : t("cards.fit.fits.you");

  const choices: { value: Priority | null; label: string; text: string }[] = [
    {
      value: null,
      label: t("cards.fit.follow"),
      text: compared ? t("cards.fit.follow.text.with", { name: compared.name, priority: priorityLabel(compared.card_priority) }) : t("cards.fit.follow.text"),
    },
    ...PRIORITY.map((p) => ({ value: p.value, label: p.label, text: p.text })),
  ];
  const current = choices.find((c) => c.value === chosen) ?? choices[0];

  return (
    <section className="set-row" data-spot="user-fit">
      {label}
      <div className="set-panel uf">
        <div className="uf-head">
          <div className={`uf-state is-${status}`} role="status">
            <span className="uf-mark" aria-hidden="true">
              <Icon name={STATE[status].icon} small />
            </span>
            <div className="uf-state-text">
              <b>{STATE[status].title()}</b>
              <p className="uf-core">{headline}</p>
              <p className="uf-sub">{sentence}</p>
            </div>
          </div>

          <div className="uf-with">
            <span className="uf-with-label" id={`uf-with-${id}`}>
              {t("cards.fit.with")}
            </span>
            <div className="uf-pick" role="radiogroup" aria-labelledby={`uf-with-${id}`}>
              {personas.map((p) => {
                const on = p.id === check.persona_id;
                return (
                  <button
                    key={p.id}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    className="uf-chip"
                    onClick={() => !on && setPick(p.id === newChatPersona?.id ? null : p.id)}
                    data-log-label="Persona zum Prüfen wählen"
                  >
                    <Portrait who={p} kind="persona" className="av" />
                    <span>{p.name}</span>
                    {p.id === newChatPersona?.id && <small>{t("cards.fit.newChats")}</small>}
                  </button>
                );
              })}
            </div>
          </div>
        </div>

        {topics.length > 0 ? (
          <ul className="uf-topics" aria-label={t("cards.fit.topics")}>
            {topics.map((tp) => (
              <TopicRow topic={tp} persona={compared} key={tp.kind} />
            ))}
          </ul>
        ) : (
          rules.length === 0 && <p className="help">{t("cards.fit.noPlace", { macro: "{{user}}" })}</p>
        )}

        {rules.length > 0 && (
          <details className="uf-rules">
            <summary data-log-label="Regeln der Karte zu dir auf-/zuklappen">
              <Icon name="chevron" small />
              <span>{t("cards.fit.rules")}</span>
              <small>{rules.length}</small>
            </summary>
            <p className="uf-rules-note">{t("cards.fit.rules.note")}</p>
            <ul>
              {rules.map((p, i) => (
                <li key={i}>
                  <span className="uf-field">{p.fields.join(" · ")}</span>
                  <p>
                    <Excerpt text={p.excerpt} />
                  </p>
                </li>
              ))}
            </ul>
          </details>
        )}

        {status === "conflict" && (
          <div className="uf-todo">
            <b>{t("cards.fit.todo")}</b>
            <ul>
              {effective !== "card" && (
                <li>
                  <span>{t("cards.fit.todo.card")}</span>
                  <button type="button" className="btn sm" onClick={() => void choose("card")}>
                    {t("cards.fit.todo.card.button")}
                  </button>
                </li>
              )}
              <li>
                <span>{t("cards.fit.todo.persona")}</span>
                <button type="button" className="btn sm" onClick={() => openSetting(`#/char/${id}/settings`, "default-persona")}>
                  {t("cards.fit.todo.persona.button")}
                </button>
              </li>
              <li>
                <span>{t("cards.fit.todo.edit")}</span>
              </li>
            </ul>
          </div>
        )}

        <div className="uf-rule">
          <span className="field-label" id={`uf-rule-${id}`}>
            {t("cards.fit.rule")}
          </span>
          <div className="uf-seg" role="radiogroup" aria-labelledby={`uf-rule-${id}`}>
            {choices.map((c) => (
              <label className="uf-seg-opt" key={c.value ?? "follow"}>
                <input type="radio" name={`user-priority-${id}`} checked={chosen === c.value} onChange={() => void choose(c.value)} />
                <span>{c.label}</span>
              </label>
            ))}
          </div>
          <p className="uf-seg-note">{current.text}</p>
          <p className={saveError ? "help bad" : "help"} role="status">
            {saveError
              ? t("cards.save.failed", { error: saveError })
              : state === "saving"
                ? t("cards.save.saving")
                : state === "saved"
                  ? t("cards.fit.saved", { name: character.name })
                  : t("cards.fit.idle", { name: character.name })}
          </p>
        </div>
      </div>
    </section>
  );
}
