// The editor of one persona.

import { useEffect, useState } from "react";
import { api, download, errorText, postJson, type Persona, type Priority } from "../api";
import { samePos } from "../Character";
import { t, texts } from "../i18n";
import { rich } from "../i18n/rich";
import type { Picked } from "../image";
import { logEvent } from "../log";
import { PictureField } from "../PictureField";
import { Icon, Portrait } from "../ui";
import { PRIORITY } from "./common";

/** Longest texts the server takes. */
const NAME_MAX = 40;
const PRONOUNS_MAX = 40;
const FIELD_MAX = 5_000;
const TEXT_MAX = 20_000;

export type Draft = {
  name: string;
  pronouns: string;
  appearance: string;
  personality: string;
  background: string;
  description: string;
  every_turn: boolean;
  card_priority: Priority;
};

const draftOf = (p: Persona | null): Draft => ({
  name: p?.name ?? "",
  pronouns: p?.pronouns ?? "",
  appearance: p?.appearance ?? "",
  personality: p?.personality ?? "",
  background: p?.background ?? "",
  description: p?.description ?? "",
  every_turn: p?.every_turn ?? false,
  card_priority: p?.card_priority ?? "persona",
});

const sameDraft = (a: Draft, b: Draft) => (Object.keys(a) as (keyof Draft)[]).every((k) => a[k] === b[k]);

type EditorProps = {
  /** null: a new persona. */
  persona: Persona | null;
  isActive: boolean;
  isLast: boolean;
  draft: Draft | undefined;
  onDraft: (d: Draft | null) => void;
  onActivate: (id: number) => void;
  onChanged: () => Promise<void>;
  onToast: (text: string) => void;
};

/** "1.234 von 5.000 Zeichen" once a text gets close to its limit. */
function Count({ n, max }: { n: number; max: number }) {
  if (n <= max * 0.9) return null;
  return (
    <span className="help num pe-count">{t("cards.persona.count", { n, max })}</span>
  );
}

type TextKey = "appearance" | "personality" | "background" | "description";

/** The texts about you, in the order the model reads them. */
const ABOUT: { key: TextKey; label: string; help: string; rows: number; max: number; wide?: boolean }[] = [
  texts({ key: "appearance", rows: 5, max: FIELD_MAX }, { label: "cards.persona.appearance", help: "cards.persona.appearance.help" }),
  texts({ key: "personality", rows: 5, max: FIELD_MAX }, { label: "cards.persona.personality", help: "cards.persona.personality.help" }),
  texts({ key: "background", rows: 4, max: FIELD_MAX, wide: true }, { label: "cards.persona.background", help: "cards.persona.background.help" }),
  texts({ key: "description", rows: 6, max: TEXT_MAX, wide: true }, { label: "cards.persona.description", help: "cards.persona.description.help" }),
];

/** When the model reads the persona. */
const WHEN: { value: boolean; label: string; text: string }[] = [
  texts({ value: false }, { label: "cards.persona.once", text: "cards.persona.once.text" }),
  texts({ value: true }, { label: "cards.persona.everyTurn", text: "cards.persona.everyTurn.text" }),
];

/** One choice of the editor as radio buttons; `standard` marks the default. */
function Choices<T extends string | boolean>(props: {
  labelledBy: string;
  spot: string;
  name: string;
  options: { value: T; label: string; text: string }[];
  standard: T;
  value: T;
  onPick: (value: T) => void;
}) {
  const { labelledBy, spot, name, options, standard, value, onPick } = props;
  return (
    <div className="choices" role="radiogroup" aria-labelledby={labelledBy} data-spot={spot}>
      {options.map((w) => (
        <label className="choice" key={String(w.value)}>
          <input type="radio" name={name} checked={value === w.value} onChange={() => onPick(w.value)} />
          <span className="choice-mark" aria-hidden="true" />
          <span className="choice-text">
            <b>
              {w.label}
              {w.value === standard && <small>{t("cards.persona.standard")}</small>}
            </b>
            <span>{w.text}</span>
          </span>
        </label>
      ))}
    </div>
  );
}

export function PersonaEditor({ persona, isActive, isLast, draft, onDraft, onActivate, onChanged, onToast }: EditorProps) {
  const fresh = persona === null;
  const saved = draftOf(persona);
  const text = draft ?? saved;
  const [state, setState] = useState<"idle" | "saving" | "saved">("idle");
  const [error, setError] = useState("");
  // Picture changes are staged like the text: nothing happens until "Speichern".
  const [picked, setPicked] = useState<Picked | null>(null);
  const [resetPicture, setResetPicture] = useState(false);
  const [pos, setPos] = useState<[number, number] | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => () => void (picked && URL.revokeObjectURL(picked.url)), [picked]);

  const base: Persona = persona ?? { id: 0, ...draftOf(null), name: text.name, has_avatar: false, avatar_rev: 0, avatar_pos: [50, 50], created_at: 0, updated_at: 0 };
  const shownPicture = { ...base, name: text.name.trim() || "?", has_avatar: !resetPicture && base.has_avatar };
  const hasPicture = !!picked || shownPicture.has_avatar;
  const shownPos = pos ?? base.avatar_pos;
  const textDirty = !sameDraft(text, saved);
  const posDirty = !!pos && !samePos(pos, base.avatar_pos);
  const dirty = textDirty || !!picked || resetPicture || posDirty;
  const name = text.name.trim();
  const pronouns = text.pronouns.trim();
  const nameOk = name.length > 0 && name.length <= NAME_MAX;

  const edit = (patch: Partial<Draft>) => {
    const next = { ...text, ...patch };
    onDraft(sameDraft(next, saved) ? null : next);
    setState("idle");
  };
  const discard = () => {
    onDraft(null);
    setPicked(null);
    setResetPicture(false);
    setPos(null);
    setError("");
  };

  const save = async () => {
    setState("saving");
    setError("");
    const body = {
      name,
      pronouns,
      appearance: text.appearance,
      personality: text.personality,
      background: text.background,
      description: text.description,
      every_turn: text.every_turn,
      card_priority: text.card_priority,
    };
    // Only lengths and the switch go into the log, never the text.
    const data = { chars: body.appearance.length + body.personality.length + body.background.length + body.description.length, every_turn: body.every_turn, card_priority: body.card_priority };
    try {
      let id = persona?.id;
      if (fresh) {
        logEvent("info", "settings", "Persona angelegt", { data });
        id = (await postJson<Persona>("/personas", body)).id;
      } else if (textDirty) {
        logEvent("info", "settings", "Persona geändert", { data: { id, ...data } });
        await postJson(`/personas/${id}`, body, "PUT");
      }
      if (picked) await api(`/personas/${id}/avatar`, { method: "PUT", body: picked.file });
      else if (resetPicture) await api(`/personas/${id}/avatar`, { method: "DELETE" });
      if (hasPicture && !resetPicture && (posDirty || (fresh && pos))) {
        await postJson(`/personas/${id}/avatar-position`, { x: shownPos[0], y: shownPos[1] }, "PUT");
      }
      onDraft(null);
      setPicked(null);
      setResetPicture(false);
      setPos(null);
      await onChanged();
      setState("saved");
      if (fresh) location.hash = `#/personas/${id}`;
    } catch (e) {
      setError(errorText(e));
      setState("idle");
      // A new persona that exists already but lost its picture on the way is still there: show it.
      if (fresh) void onChanged();
    }
  };

  const [exporting, setExporting] = useState(false);
  const exportPng = async () => {
    if (!persona) return;
    setExporting(true);
    try {
      const file = await download(`/personas/${persona.id}/export.png`, {}, `${persona.name}.png`);
      if (file !== null) onToast(t("cards.savedFile", { file }));
    } catch (e) {
      onToast(t("cards.exportFailed", { error: errorText(e) }));
    } finally {
      setExporting(false);
    }
  };

  const remove = async () => {
    if (!persona) return;
    logEvent("info", "settings", "Persona gelöscht", { data: { id: persona.id, was_active: isActive } });
    try {
      await api(`/personas/${persona.id}`, { method: "DELETE" });
      onDraft(null);
      await onChanged();
      onToast(t("cards.persona.deleted", { name: persona.name }));
      location.hash = "#/personas";
    } catch (e) {
      setConfirming(false);
      setError(errorText(e));
    }
  };

  return (
    <form
      className="pe"
      onSubmit={(e) => {
        e.preventDefault();
        if (dirty && nameOk && state !== "saving") void save();
      }}
    >
      <div className="pe-title">
        <h2>
          {fresh ? t("cards.persona.list.new") : name || saved.name}
          {!fresh && pronouns && <small>{pronouns}</small>}
        </h2>
        {!fresh && (
          <button type="button" className="btn sm" onClick={() => void exportPng()} disabled={exporting} title={t("cards.persona.exportPng.title")}>
            <Icon name="download" small />
            {t("cards.persona.exportPng")}
          </button>
        )}
        {!fresh &&
          (isActive ? (
            <span className="tag pl-on">
              <Icon name="check" small />
              {t("cards.persona.playing", { name: saved.name })}
            </span>
          ) : (
            <button type="button" className="btn sm solid" onClick={() => onActivate(persona.id)} data-log-label="Als Persona spielen">
              {t("cards.persona.play", { name: saved.name })}
            </button>
          ))}
      </div>

      <section className="pe-sec" aria-labelledby="pe-basics">
        <header className="pe-sec-head">
          <h3 id="pe-basics">{t("cards.persona.basics")}</h3>
          <p>{t("cards.persona.basics.text")}</p>
        </header>
        <PictureField
          who={shownPicture}
          kind="persona"
          picked={picked}
          pos={shownPos}
          onPos={setPos}
          hasPicture={hasPicture}
          onPick={(p) => {
            setPicked(p);
            setResetPicture(false);
            setPos(p.pos);
            setState("idle");
          }}
          onDiscard={() => {
            setPicked(null);
            setPos(null);
          }}
          removeLabel={base.has_avatar && !resetPicture ? t("cards.persona.picture.remove") : null}
          onRemove={() => setResetPicture(true)}
          cropHelp={t("cards.persona.picture.crop")}
          spot="persona-picture"
          previews={
            <>
              <Portrait who={shownPicture} className="pv-round" preview={picked?.url} pos={shownPos} kind="persona" />
              <Portrait who={shownPicture} className="pv-mini" preview={picked?.url} pos={shownPos} kind="persona" />
            </>
          }
        />

        <div className="pe-pair">
          <label className="field" data-spot="persona-name">
            <span className="field-label">{t("cards.persona.name")}</span>
            <span className="help">{rich("cards.persona.name.help", { macro: <code className="macro">{"{{user}}"}</code>, name: name || "…" })}</span>
            <input className="input" value={text.name} maxLength={NAME_MAX} required autoFocus={fresh} onChange={(e) => edit({ name: e.target.value })} />
          </label>
          <label className="field" data-spot="persona-pronouns">
            <span className="field-label">
              {t("cards.persona.pronouns")} <small>{t("cards.persona.optional")}</small>
            </span>
            <span className="help">{t("cards.persona.pronouns.help")}</span>
            <input
              className="input"
              value={text.pronouns}
              maxLength={PRONOUNS_MAX}
              placeholder={t("cards.persona.pronouns.placeholder")}
              list="pronoun-ideas"
              autoComplete="off"
              onChange={(e) => edit({ pronouns: e.target.value })}
            />
            <datalist id="pronoun-ideas">
              {t("cards.persona.pronouns.ideas")
                .split("|")
                .map((idea) => (
                  <option key={idea} value={idea} />
                ))}
            </datalist>
          </label>
        </div>
      </section>

      <section className="pe-sec" aria-labelledby="pe-about">
        <header className="pe-sec-head">
          <h3 id="pe-about">{t("cards.persona.about")}</h3>
          <p>{t("cards.persona.about.text")}</p>
        </header>
        <div className="pe-fields">
          {ABOUT.map((f) => (
            <label className={`field${f.wide ? " wide" : ""}`} key={f.key} data-spot={`persona-${f.key}`}>
              <span className="field-label">{f.label}</span>
              <span className="help">{f.help}</span>
              <textarea className="textarea" rows={f.rows} value={text[f.key]} maxLength={f.max} onChange={(e) => edit({ [f.key]: e.target.value })} />
              <Count n={text[f.key].length} max={f.max} />
            </label>
          ))}
        </div>
      </section>

      <section className="pe-sec" aria-labelledby="pe-when">
        <header className="pe-sec-head">
          <h3 id="pe-when">{t("cards.persona.when")}</h3>
          <p>{name ? t("cards.persona.when.text", { name }) : t("cards.persona.when.text.unnamed")}</p>
        </header>
        <Choices
          labelledBy="pe-when"
          spot="persona-every-turn"
          name="every-turn"
          options={WHEN}
          standard={false}
          value={text.every_turn}
          onPick={(every_turn) => edit({ every_turn })}
        />
      </section>

      <section className="pe-sec" aria-labelledby="pe-priority">
        <header className="pe-sec-head">
          <h3 id="pe-priority">{t("cards.persona.priority")}</h3>
          <p>{t("cards.persona.priority.text")}</p>
        </header>
        <Choices
          labelledBy="pe-priority"
          spot="persona-card-priority"
          name="card-priority"
          options={PRIORITY}
          standard="persona"
          value={text.card_priority}
          onPick={(card_priority) => edit({ card_priority })}
        />
      </section>

      {!fresh && (
        <section className="pe-block pe-danger" aria-label={t("cards.persona.delete")}>
          <div>
            <span className="field-label">{t("cards.persona.delete")}</span>
            <p className="help">
              {isLast ? t("cards.persona.delete.last") : isActive ? t("cards.persona.delete.active") : t("cards.persona.delete.other")}
            </p>
          </div>
          {confirming ? (
            <div className="confirm" role="group" aria-label={t("cards.persona.delete.confirm")}>
              <button type="button" className="btn sm danger" onClick={() => void remove()} autoFocus>
                {t("cards.persona.delete.final")}
              </button>
              <button type="button" className="btn sm ghost" onClick={() => setConfirming(false)}>
                {t("cards.cancel")}
              </button>
            </div>
          ) : (
            <button
              type="button"
              className="btn sm danger"
              disabled={isLast}
              title={isLast ? t("cards.persona.delete.lastTitle") : undefined}
              onClick={() => setConfirming(true)}
            >
              <Icon name="trash" small />
              {t("cards.deleteName", { name: saved.name })}
            </button>
          )}
        </section>
      )}

      <div className={`savebar pe-save${dirty ? " dirty" : ""}`}>
        <span className={error ? "bad" : "muted"} role="status">
          {error
            ? t("cards.save.failed", { error })
            : state === "saving"
              ? t("cards.save.saving")
              : dirty
                ? fresh
                  ? t("cards.persona.notCreated")
                  : t("cards.unsaved")
                : state === "saved"
                  ? t("cards.save.saved")
                  : fresh
                    ? t("cards.persona.needName")
                    : t("cards.save.none")}
        </span>
        <button type="button" className="btn ghost" onClick={discard} disabled={!dirty || state === "saving"}>
          {t("cards.save.discard")}
        </button>
        <button type="submit" className="btn primary" disabled={!dirty || !nameOk || state === "saving"}>
          {fresh ? t("cards.persona.create") : t("cards.save.save")}
        </button>
      </div>
    </form>
  );
}
