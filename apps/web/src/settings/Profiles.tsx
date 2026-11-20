// Settings tab "Experte" → "Profile": every model profile with activate, edit, duplicate, rename and
// delete, and a new profile from the active one with a model from "Meine Modelle".

import { useId, useState, type FormEvent } from "react";
import { ctxShort, errorText, gb, num, postJson, api, type EngineInfo, type LocalModel, type Profile } from "../api";
import { t } from "../i18n";
import { Icon } from "../ui";
import { draftOf } from "./args";
import { localName, ORIGIN_TEXT } from "./LocalModels";
import { SwitchProgress, type Progress } from "./switch";

export const fileOf = (path: string) => path.split(/[\\/]/).pop() ?? path;
const NAME_MAX = 60;

/** The context from start options (`-c N`, `--ctx-size N`, `-c=N`); null: not set, 0: the model's own. */
export function ctxOfArgs(args: string[]): number | null {
  const v = Number(draftOf(args).ctx ?? NaN);
  return Number.isInteger(v) && v >= 0 ? v : null;
}

/** "Temperatur 0,8 · 800 Tokens · Denken an": the profile's answer settings in one line. */
function samplerSummary(p: Profile) {
  const s = p.sampler;
  const parts = [
    t("models.profile.temperature", { v: num(s.temperature ?? 0.8) }),
    t("models.advice.tokens", { n: s.n_predict ?? 800 }),
    p.reasoning === "on" ? t("models.profile.thinking") : null,
  ];
  return parts.filter(Boolean).join(" · ");
}

type ListProps = {
  profiles: Profile[];
  engine: EngineInfo | null;
  progress: Progress | null;
  switching: boolean;
  /** The profile the editor below shows. */
  editing: number | null;
  /** The editor holds unsaved changes: no other profile can be opened in it. */
  locked: boolean;
  models: LocalModel[] | null;
  onActivate: (p: Profile) => void;
  onEdit: (id: number) => void;
  onChanged: () => Promise<void>;
};

export function ProfileList({ profiles, engine, progress, switching, editing, locked, models, onActivate, onEdit, onChanged }: ListProps) {
  const active = profiles.find((p) => p.active) ?? null;
  const firstOther = profiles.find((x) => !x.active)?.id;
  const duplicate = async (p: Profile) => {
    const made = await postJson<Profile>("/models/profiles", {
      name: t("models.profile.copyName", { name: p.name }).slice(0, NAME_MAX),
      engine_id: p.engine_id,
      model_file: p.model_file,
      load_args: p.load_args,
      sampler: p.sampler,
      reasoning: p.reasoning,
    });
    await onChanged();
    if (!locked) onEdit(made.id);
  };
  return (
    <>
      {profiles.length === 0 && <p className="help">{t("models.profile.none")}</p>}
      <ul className="mp-list" aria-label={t("models.profiles")}>
        {profiles.map((p) => (
          <ProfileRow
            key={p.id}
            p={p}
            first={p.id === firstOther}
            editing={p.id === editing}
            locked={locked}
            ctxNow={p.active ? (engine?.model.ctx ?? null) : null}
            progress={progress?.id === p.id ? progress : null}
            engine={engine}
            switching={switching}
            onActivate={() => onActivate(p)}
            onEdit={() => onEdit(p.id)}
            onDuplicate={() => duplicate(p)}
            onChanged={onChanged}
          />
        ))}
      </ul>
      <NewProfile from={active} models={models} onCreated={async (id) => {
        await onChanged();
        if (!locked) onEdit(id);
      }} />
    </>
  );
}

type RowProps = {
  p: Profile;
  /** The first profile one can switch to: Ctrl+K "Profil wechseln" puts the focus on its button. */
  first: boolean;
  editing: boolean;
  locked: boolean;
  /** The context the running server uses (active profile only). */
  ctxNow: number | null;
  progress: Progress | null;
  engine: EngineInfo | null;
  switching: boolean;
  onActivate: () => void;
  onEdit: () => void;
  onDuplicate: () => Promise<void>;
  onChanged: () => Promise<void>;
};

function ProfileRow({ p, first, editing, locked, ctxNow, progress, engine, switching, onActivate, onEdit, onDuplicate, onChanged }: RowProps) {
  const [mode, setMode] = useState<"idle" | "rename" | "delete">("idle");
  const [error, setError] = useState("");
  const ctx = ctxOfArgs(p.load_args);
  const ctxText = ctx ? t("models.find.ctx", { ctx: ctxShort(ctx) }) : ctxNow ? t("models.find.ctx", { ctx: ctxShort(ctxNow) }) : t("models.profile.modelCtx");

  const remove = async () => {
    try {
      await api(`/models/profiles/${p.id}`, { method: "DELETE" });
      await onChanged();
    } catch (e) {
      setError(t("models.local.notDeleted", { error: errorText(e) }));
      setMode("idle");
    }
  };
  const duplicate = async () => {
    try {
      setError("");
      await onDuplicate();
    } catch (e) {
      setError(t("models.profile.notDuplicated", { error: errorText(e) }));
    }
  };

  return (
    <li className={`mp-row${p.active ? " on" : ""}${editing ? " mx-editing" : ""}`} aria-label={t("models.profile.row", { name: p.name })}>
      <div className="mp-top">
        <div className="mp-text">
          <span className="mp-title">
            <b>{p.name}</b>
            {p.active && <span className="tag tag-on">{t("models.local.activeTag")}</span>}
            <span className={`tag${p.measured ? " tag-ok" : ""}`} title={p.measured ? t("models.measured.title") : t("models.guessed.title")}>
              {p.measured ? t("models.measured") : t("models.guessed")}
            </span>
            {editing && <span className="tag mx-tag-edit">{t("models.profile.editing")}</span>}
          </span>
          <span className="mp-meta num" title={p.model_file}>
            <span className="mp-file">{fileOf(p.model_file)}</span> · {ctxText} · {samplerSummary(p)}
          </span>
        </div>
        {mode === "idle" && (
          <div className="mp-actions">
            {!p.active && (
              <button type="button" className="btn sm" onClick={onActivate} disabled={switching} data-spot-focus={first || undefined}>
                {t("models.profile.activate")}
              </button>
            )}
            {!editing && (
              <button
                type="button"
                className="btn sm ghost"
                onClick={onEdit}
                disabled={locked}
                title={locked ? t("models.profile.locked") : undefined}
                aria-label={t("models.profile.edit.aria", { name: p.name })}
              >
                <Icon name="sliders" small />
                {t("models.profile.edit")}
              </button>
            )}
            <button type="button" className="btn sm ghost" onClick={() => void duplicate()} aria-label={t("models.profile.duplicate.aria", { name: p.name })}>
              <Icon name="copy" small />
              {t("models.profile.duplicate")}
            </button>
            <button type="button" className="btn sm ghost" onClick={() => setMode("rename")}>
              <Icon name="pencil" small />
              {t("models.profile.rename")}
            </button>
            {!p.active && (
              <button type="button" className="btn sm ghost" onClick={() => setMode("delete")} aria-label={t("models.local.delete.aria", { name: p.name })}>
                <Icon name="trash" small />
                {t("models.local.delete")}
              </button>
            )}
          </div>
        )}
      </div>
      {progress && <SwitchProgress progress={progress} engine={engine} />}
      {mode === "rename" && <Rename p={p} onDone={() => setMode("idle")} onChanged={onChanged} />}
      {mode === "delete" && (
        <div className="mp-confirm" role="group" aria-label={t("models.local.confirm.aria")}>
          <p>{t("models.profile.confirm", { name: p.name })}</p>
          <div className="btn-row">
            <button type="button" className="btn sm danger" onClick={() => void remove()} autoFocus>
              {t("models.profile.confirm.yes")}
            </button>
            <button type="button" className="btn sm ghost" onClick={() => setMode("idle")}>
              {t("models.dl.cancel")}
            </button>
          </div>
        </div>
      )}
      {error && <p className="bad mp-error">{error}</p>}
    </li>
  );
}

function Rename({ p, onDone, onChanged }: { p: Profile; onDone: () => void; onChanged: () => Promise<void> }) {
  const [name, setName] = useState(p.name);
  const [error, setError] = useState("");
  const id = useId();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const next = name.trim();
    if (next === p.name) return onDone();
    try {
      await postJson(`/models/profiles/${p.id}`, { name: next }, "PUT");
      await onChanged();
      onDone();
    } catch (err) {
      setError(errorText(err));
    }
  };
  return (
    <form className="mp-form" aria-label={t("models.profile.rename")} onSubmit={(e) => void submit(e)} onKeyDown={(e) => e.key === "Escape" && onDone()}>
      <label className="field-label" htmlFor={id}>
        {t("models.profile.newName")}
      </label>
      <div className="mp-inline">
        <input id={id} className="input" value={name} maxLength={NAME_MAX} autoFocus onChange={(e) => setName(e.target.value)} />
        <button type="submit" className="btn sm primary" disabled={!name.trim()}>
          {t("models.save")}
        </button>
        <button type="button" className="btn sm ghost" onClick={onDone}>
          {t("models.dl.cancel")}
        </button>
      </div>
      {error && <p className="bad">{error}</p>}
    </form>
  );
}

/** The model files one can choose for a profile, as options; `keep` stays a choice even outside the model folders. */
export function ModelOptions({ models, keep }: { models: LocalModel[] | null; keep: string }) {
  const list = (models ?? []).filter((m) => m.missing_parts.length === 0);
  const outside = keep && !list.some((m) => m.path.toLowerCase() === keep.toLowerCase());
  return (
    <>
      {outside && <option value={keep}>{fileOf(keep)}</option>}
      {list.map((m) => (
        <option key={m.path} value={m.path}>
          {[localName(m), m.quant, gb(m.size_bytes), ORIGIN_TEXT[m.origin]].filter(Boolean).join(" · ")}
        </option>
      ))}
    </>
  );
}

/** "Neues Profil": a copy of the active one with its own name and a model from "Meine Modelle". */
function NewProfile({ from, models, onCreated }: { from: Profile | null; models: LocalModel[] | null; onCreated: (id: number) => Promise<void> }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [file, setFile] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const base = useId();

  if (!from) return null;
  const start = () => {
    setName(t("models.profile.copyName", { name: from.name }).slice(0, NAME_MAX));
    setFile(from.model_file);
    setError("");
    setOpen(true);
  };
  if (!open) {
    return (
      <div className="btn-row">
        <button type="button" className="btn sm" onClick={start} data-spot="profile-new">
          <Icon name="plus" small />
          {t("models.profile.new")}
        </button>
      </div>
    );
  }
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    try {
      const made = await postJson<Profile>("/models/profiles", {
        name: name.trim(),
        engine_id: from.engine_id,
        model_file: file,
        load_args: from.load_args,
        sampler: from.sampler,
        reasoning: from.reasoning,
      });
      setOpen(false);
      await onCreated(made.id);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setSaving(false);
    }
  };
  return (
    <form className="mp-new" onSubmit={(e) => void submit(e)} onKeyDown={(e) => e.key === "Escape" && setOpen(false)} aria-label={t("models.profile.new")}>
      <h3>{t("models.profile.new")}</h3>
      <p className="help">{t("models.profile.new.note", { name: from.name })}</p>
      <div className="field">
        <label className="field-label" htmlFor={`${base}-name`}>
          {t("models.profile.name")}
        </label>
        <input id={`${base}-name`} className="input" value={name} maxLength={NAME_MAX} autoFocus onChange={(e) => setName(e.target.value)} />
      </div>
      <div className="field">
        <label className="field-label" htmlFor={`${base}-file`}>
          {t("models.profile.model")} <small>{t("models.profile.fromMine")}</small>
        </label>
        <select id={`${base}-file`} className="input ms-select" value={file} onChange={(e) => setFile(e.target.value)}>
          <ModelOptions models={models} keep={from.model_file} />
        </select>
      </div>
      {error && <p className="bad">{error}</p>}
      <div className="btn-row">
        <button type="submit" className="btn sm primary" disabled={saving || !name.trim() || !file}>
          {saving ? t("models.profile.creating") : t("models.profile.create")}
        </button>
        <button type="button" className="btn sm ghost" onClick={() => setOpen(false)}>
          {t("models.dl.cancel")}
        </button>
      </div>
    </form>
  );
}
