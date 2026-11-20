// Step 2 of the card builder: every field of the card. Each field the local model can write has a
// button for it ("Schreiben lassen", or "Neu schreiben" once it holds text) and, after a rewrite, the
// way back to the text before.

import type { ReactNode } from "react";
import { t } from "../i18n";
import { rich } from "../i18n/rich";
import type { Picked } from "../image";
import { PictureField } from "../PictureField";
import { Icon, Portrait } from "../ui";
import { emptyLore, filled, tokens, type Draft, type Field, type LoreEntry } from "./draft";

export type Writing = { field: Field; all: boolean } | null;

type WriteProps = {
  field: Field;
  draft: Draft;
  writing: Writing;
  onWrite: (f: Field) => void;
  onStop: () => void;
  /** The text before the last rewrite, if there is one. */
  undo?: boolean;
  onUndo: (f: Field) => void;
  /** Overrides the button's text (one more greeting). */
  label?: string;
};

/** The model's button of one field. */
function Write({ field, draft, writing, onWrite, onStop, undo, onUndo, label }: WriteProps) {
  const mine = writing?.field === field;
  return (
    <div className="cb-write">
      {mine ? (
        <>
          <span className="cb-busy" role="status">
            <i className="cb-dot" aria-hidden="true" />
            {t("builder.write.busy")}
          </span>
          <button type="button" className="btn sm ghost" onClick={onStop}>
            {t("builder.write.stop")}
          </button>
        </>
      ) : (
        <button type="button" className="btn sm" disabled={writing !== null} onClick={() => onWrite(field)}>
          <Icon name="pencil" small />
          {label ?? (filled(draft, field) ? t("builder.write.again") : t("builder.write.one"))}
        </button>
      )}
      {undo && !mine && (
        <button type="button" className="btn sm ghost" disabled={writing !== null} onClick={() => onUndo(field)}>
          <Icon name="rewind" small />
          {t("builder.write.undo")}
        </button>
      )}
    </div>
  );
}

type TextProps = {
  label: string;
  tech?: string;
  help: ReactNode;
  value: string;
  rows: number;
  onChange: (v: string) => void;
  write?: ReactNode;
  spot?: string;
};

/** A text field with its help, its size in tokens and the model's button under it. */
function TextField({ label, tech, help, value, rows, onChange, write, spot }: TextProps) {
  const n = tokens(value);
  return (
    <div className="field" data-spot={spot}>
      <label className="field-label" htmlFor={spot}>
        {label}
        {tech && <small> {tech}</small>}
      </label>
      <span className="help">{help}</span>
      <textarea id={spot} className="textarea" rows={rows} value={value} onChange={(e) => onChange(e.target.value)} />
      <div className="cb-under">
        {write}
        {n > 0 && <span className="cb-count num">{t("builder.tokens", { n })}</span>}
      </div>
    </div>
  );
}

type Props = {
  draft: Draft;
  onDraft: (patch: Partial<Draft>) => void;
  writing: Writing;
  onWrite: (f: Field) => void;
  onStop: () => void;
  undo: Partial<Record<Field, unknown>>;
  onUndo: (f: Field) => void;
  picked: Picked | null;
  pos: [number, number];
  onPick: (p: Picked | null) => void;
  onPos: (pos: [number, number]) => void;
};

export function Fields({ draft, onDraft, writing, onWrite, onStop, undo, onUndo, picked, pos, onPick, onPos }: Props) {
  const w = (field: Field, label?: string) => <Write field={field} draft={draft} writing={writing} onWrite={onWrite} onStop={onStop} undo={field in undo} onUndo={onUndo} label={label} />;
  const who = { id: 0, name: draft.name || "?", has_avatar: !!picked, avatar_rev: 0, avatar_pos: pos };
  const greetings = draft.alternate_greetings;
  const lore = draft.lore;
  const editLore = (i: number, patch: Partial<LoreEntry>) => onDraft({ lore: lore.map((l, j) => (j === i ? { ...l, ...patch } : l)) });
  const macro = (s: string) => <code>{s}</code>;

  return (
    <>
      <div className="well cb-macros">
        <p>{rich("builder.macros", { char: macro("{{char}}"), user: macro("{{user}}") })}</p>
      </div>

      <section className="set-row">
        <div className="set-label">
          <h2>{t("builder.sec.profile")}</h2>
          <p>{t("builder.sec.profile.text")}</p>
        </div>
        <div className="set-panel">
          <PictureField
            who={who}
            picked={picked}
            pos={pos}
            onPos={onPos}
            hasPicture={!!picked}
            onPick={onPick}
            onDiscard={() => onPick(null)}
            removeLabel={null}
            onRemove={() => onPick(null)}
            cropHelp={t("builder.pic.crop")}
            spot="builder-picture"
            previews={
              <>
                <Portrait who={who} className="pv-card" preview={picked?.url} pos={pos} full />
                <Portrait who={who} className="pv-round" preview={picked?.url} pos={pos} full />
              </>
            }
          />
          <div className="field" data-spot="builder-name">
            <label className="field-label" htmlFor="cb-name">
              {t("builder.f.name")}
            </label>
            <input id="cb-name" className="input" value={draft.name} maxLength={120} onChange={(e) => onDraft({ name: e.target.value })} />
            <div className="cb-under">{w("name")}</div>
          </div>
          <div className="field">
            <label className="field-label" htmlFor="cb-tags">
              {t("builder.f.tags")}
            </label>
            <span className="help">{t("builder.f.tags.help")}</span>
            <input
              id="cb-tags"
              className="input"
              value={draft.tags.join(", ")}
              onChange={(e) => onDraft({ tags: e.target.value.split(",").map((x) => x.trimStart()) })}
              onBlur={() => onDraft({ tags: draft.tags.map((x) => x.trim()).filter(Boolean) })}
            />
            <div className="cb-under">{w("tags")}</div>
          </div>
          <TextField label={t("builder.f.creator_notes")} help={t("builder.f.creator_notes.help")} value={draft.creator_notes} rows={2} onChange={(creator_notes) => onDraft({ creator_notes })} write={w("creator_notes")} spot="cb-notes" />
          <div className="cb-two">
            <label className="field">
              <span className="field-label">{t("builder.f.creator")}</span>
              <input className="input" value={draft.creator} maxLength={80} onChange={(e) => onDraft({ creator: e.target.value })} />
            </label>
            <label className="field">
              <span className="field-label">{t("builder.f.version")}</span>
              <input className="input" value={draft.version} maxLength={20} placeholder="1.0" onChange={(e) => onDraft({ version: e.target.value })} />
            </label>
          </div>
        </div>
      </section>

      <section className="set-row">
        <div className="set-label">
          <h2>{t("builder.sec.figure")}</h2>
          <p>{t("builder.sec.figure.text")}</p>
        </div>
        <div className="set-panel">
          <TextField label={t("builder.f.description")} tech="description" help={t("builder.f.description.help", { user: "{{user}}" })} value={draft.description} rows={10} onChange={(description) => onDraft({ description })} write={w("description")} spot="cb-description" />
          <TextField label={t("builder.f.personality")} tech="personality" help={t("builder.f.personality.help")} value={draft.personality} rows={2} onChange={(personality) => onDraft({ personality })} write={w("personality")} spot="cb-personality" />
          <TextField label={t("builder.f.scenario")} tech="scenario" help={t("builder.f.scenario.help")} value={draft.scenario} rows={3} onChange={(scenario) => onDraft({ scenario })} write={w("scenario")} spot="cb-scenario" />
        </div>
      </section>

      <section className="set-row">
        <div className="set-label">
          <h2>{t("builder.sec.greeting")}</h2>
          <p>{t("builder.sec.greeting.text")}</p>
        </div>
        <div className="set-panel">
          <TextField label={t("builder.f.first_mes")} tech="first_mes" help={t("builder.f.first_mes.help", { user: "{{user}}" })} value={draft.first_mes} rows={7} onChange={(first_mes) => onDraft({ first_mes })} write={w("first_mes")} spot="cb-first" />
          <div className="field">
            <span className="field-label">{t("builder.f.greetings")}</span>
            <span className="help">{t("builder.f.greetings.help")}</span>
            {greetings.map((g, i) => (
              <div className="greet" key={i}>
                <textarea
                  className="textarea"
                  rows={4}
                  value={g}
                  aria-label={t("builder.f.greetings.one", { n: i + 1 })}
                  onChange={(e) => onDraft({ alternate_greetings: greetings.map((x, j) => (j === i ? e.target.value : x)) })}
                />
                <button type="button" className="icon-btn" aria-label={t("builder.f.greetings.remove", { n: i + 1 })} onClick={() => onDraft({ alternate_greetings: greetings.filter((_, j) => j !== i) })}>
                  <Icon name="trash" small />
                </button>
              </div>
            ))}
            <div className="cb-under">
              <button type="button" className="btn sm" onClick={() => onDraft({ alternate_greetings: [...greetings, ""] })}>
                <Icon name="plus" small />
                {t("builder.f.greetings.add")}
              </button>
              {w("alt_greeting", t("builder.write.greeting"))}
            </div>
          </div>
          <TextField label={t("builder.f.mes_example")} tech="mes_example" help={t("builder.f.mes_example.help", { user: "{{user}}", char: "{{char}}" })} value={draft.mes_example} rows={6} onChange={(mes_example) => onDraft({ mes_example })} write={w("mes_example")} spot="cb-examples" />
        </div>
      </section>

      <section className="set-row">
        <div className="set-label">
          <h2>{t("builder.sec.lore")}</h2>
          <p>{t("builder.sec.lore.text")}</p>
        </div>
        <div className="set-panel" data-spot="builder-lore">
          {lore.length === 0 && <p className="help">{t("builder.lore.empty")}</p>}
          {lore.map((l, i) => (
            <fieldset className="cb-lore" key={i}>
              <legend className="cb-lore-head">
                <span>{l.name.trim() || t("builder.lore.entry", { n: i + 1 })}</span>
              </legend>
              <div className="cb-two">
                <label className="field">
                  <span className="field-label">{t("builder.lore.name")}</span>
                  <input className="input" value={l.name} onChange={(e) => editLore(i, { name: e.target.value })} />
                </label>
                <label className="field">
                  <span className="field-label">{t("builder.lore.keys")}</span>
                  <input
                    className="input"
                    value={l.keys.join(", ")}
                    placeholder={t("builder.lore.keys.example")}
                    onChange={(e) => editLore(i, { keys: e.target.value.split(",").map((x) => x.trimStart()) })}
                    onBlur={() => editLore(i, { keys: l.keys.map((x) => x.trim()).filter(Boolean) })}
                  />
                </label>
              </div>
              <label className="field">
                <span className="field-label">{t("builder.lore.content")}</span>
                <textarea className="textarea" rows={3} value={l.content} onChange={(e) => editLore(i, { content: e.target.value })} />
              </label>
              <div className="cb-under">
                <label className="cb-check">
                  <input type="checkbox" checked={l.constant} onChange={(e) => editLore(i, { constant: e.target.checked })} />
                  {t("builder.lore.constant")}
                </label>
                <label className="cb-check">
                  <input type="checkbox" checked={l.enabled} onChange={(e) => editLore(i, { enabled: e.target.checked })} />
                  {t("builder.lore.enabled")}
                </label>
                <button type="button" className="btn sm ghost cb-push" onClick={() => onDraft({ lore: lore.filter((_, j) => j !== i) })}>
                  <Icon name="trash" small />
                  {t("builder.lore.remove")}
                </button>
              </div>
            </fieldset>
          ))}
          <div className="btn-row">
            <button type="button" className="btn sm" onClick={() => onDraft({ lore: [...lore, emptyLore()] })}>
              <Icon name="plus" small />
              {t("builder.lore.add")}
            </button>
          </div>
        </div>
      </section>

      <section className="set-row">
        <div className="set-label">
          <h2>{t("builder.sec.extra")}</h2>
          <p>{t("builder.sec.extra.text")}</p>
        </div>
        <div className="set-panel">
          <TextField label={t("builder.f.system_prompt")} tech="system_prompt" help={t("builder.f.system_prompt.help")} value={draft.system_prompt} rows={3} onChange={(system_prompt) => onDraft({ system_prompt })} spot="cb-system" />
          <TextField
            label={t("builder.f.post_history")}
            tech="post_history_instructions"
            help={t("builder.f.post_history.help")}
            value={draft.post_history_instructions}
            rows={2}
            onChange={(post_history_instructions) => onDraft({ post_history_instructions })}
            spot="cb-post"
          />
        </div>
      </section>
    </>
  );
}
