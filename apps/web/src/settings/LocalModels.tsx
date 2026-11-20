// Settings tab "Modell" → "Meine Modelle": the model files in the app's model folder, LM Studio's
// folder and the folders the user added. Each can be used (its profile is activated, or one is
// made), shown in Explorer, and, in the app's own folder, deleted. Below: the model folders.

import { useId, useState, type FormEvent } from "react";
import { ctxShort, errorText, gb, paramsShort, postJson, api, type EngineInfo, type LocalModel, type LocalModels, type ModelFolders, type ModelOrigin } from "../api";
import { t, texts } from "../i18n";
import { onOtherDevice } from "../phone/shell";
import { Icon } from "../ui";
import { SwitchProgress, type Progress } from "./switch";

/** Read in the language on screen each time (`texts`). */
export const ORIGIN_TEXT: Record<ModelOrigin, string> = texts({ lmstudio: "LM Studio" }, { app: "models.local.origin.app", extra: "models.local.origin.extra" });
/** From this many models on, the list gets a search and a filter. */
const FILTER_FROM = 8;

export const localName = (m: LocalModel) => m.name || m.file.replace(/\.gguf$/i, "");

type ListProps = {
  data: LocalModels | null;
  error: string;
  progress: Progress | null;
  /** The model a "Benutzen" is running for. */
  usePath: string | null;
  engine: EngineInfo | null;
  switching: boolean;
  onUse: (m: LocalModel) => void;
  onChanged: () => Promise<void>;
};

export function LocalModelList({ data, error, progress, usePath, engine, switching, onUse, onChanged }: ListProps) {
  const [query, setQuery] = useState("");
  const [origin, setOrigin] = useState<ModelOrigin | "all">("all");
  if (!data) {
    return error ? (
      <p className="bad">{error}</p>
    ) : (
      <div aria-busy="true">
        <span className="bar-ph" />
        <span className="bar-ph short" />
      </div>
    );
  }
  const all = data.models;
  const filtering = all.length >= FILTER_FROM;
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const shown = all.filter((m) => {
    if (filtering && origin !== "all" && m.origin !== origin) return false;
    const hay = [localName(m), m.file, m.quant, m.arch, ORIGIN_TEXT[m.origin]].join(" ").toLowerCase();
    return words.every((w) => hay.includes(w));
  });
  const origins = (["app", "lmstudio", "extra"] as ModelOrigin[]).filter((o) => all.some((m) => m.origin === o));
  return (
    <>
      {error && <p className="bad">{error}</p>}
      {filtering && (
        <div className="lm-filter">
          <label className="search">
            <Icon name="search" small />
            <input className="input" type="search" placeholder={t("models.local.search")} aria-label={t("models.local.search")} value={query} onChange={(e) => setQuery(e.target.value)} />
          </label>
          {origins.length > 1 && (
            <div className="seg" role="group" aria-label={t("models.local.folder")}>
              {(["all", ...origins] as const).map((o) => (
                <button key={o} type="button" aria-pressed={origin === o} onClick={() => setOrigin(o)}>
                  {o === "all" ? t("models.filter.all") : ORIGIN_TEXT[o]}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
      {all.length === 0 ? (
        <p className="help">{t("models.local.none")}</p>
      ) : shown.length === 0 ? (
        <p className="help">{t("models.local.noHit")}</p>
      ) : (
        <ul className="mp-list" aria-label={t("models.tab.mine")}>
          {shown.map((m) => (
            <ModelRow key={m.path} m={m} progress={usePath === m.path ? progress : null} engine={engine} switching={switching} onUse={() => onUse(m)} onChanged={onChanged} />
          ))}
        </ul>
      )}
      <p className="help num">
        {t("models.local.count", { n: all.length })} · {gb(all.reduce((n, m) => n + m.size_bytes, 0))}
      </p>
    </>
  );
}

type RowProps = { m: LocalModel; progress: Progress | null; engine: EngineInfo | null; switching: boolean; onUse: () => void; onChanged: () => Promise<void> };

function ModelRow({ m, progress, engine, switching, onUse, onChanged }: RowProps) {
  const [confirm, setConfirm] = useState(false);
  const [error, setError] = useState("");
  const name = localName(m);
  const running = m.profiles.some((p) => p.active);
  const broken = m.missing_parts.length > 0;
  const params =
    m.params_total === null
      ? null
      : m.moe && m.params_active !== null && m.params_active !== m.params_total
        ? `${paramsShort(m.params_total)} · ${t("models.local.active", { params: paramsShort(m.params_active) })}`
        : paramsShort(m.params_total);
  const facts = [m.quant, gb(m.size_bytes), params, m.ctx_train ? t("models.advice.ctx", { ctx: ctxShort(m.ctx_train) }) : null].filter(Boolean).join(" · ");
  const users = m.profiles.map((p) => p.name).join(", ");

  const reveal = async () => {
    try {
      await postJson("/models/local/reveal", { path: m.path });
      setError("");
    } catch (e) {
      setError(errorText(e));
    }
  };
  const remove = async () => {
    try {
      await api("/models/local", { method: "DELETE", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: m.path }) });
      await onChanged();
    } catch (e) {
      setError(t("models.local.notDeleted", { error: errorText(e) }));
      setConfirm(false);
    }
  };

  return (
    <li className={`mp-row lm-row${running ? " on" : ""}`} aria-label={t("models.local.row", { name })}>
      <div className="mp-top">
        <div className="mp-text">
          <span className="mp-title">
            <b>{name}</b>
            {running && <span className="tag tag-on">{t("models.local.activeTag")}</span>}
            {m.parts > 1 && <span className="tag">{t("models.tp.parts", { n: m.parts })}</span>}
            {m.mmproj.length > 0 && (
              <span className="tag" title={m.mmproj.join(", ")}>
                {t("models.local.mmproj")}
              </span>
            )}
          </span>
          <span className="mp-meta num">{m.readable ? facts : t("models.local.unreadable", { size: gb(m.size_bytes) })}</span>
          <span className="lm-where" title={m.path}>
            {ORIGIN_TEXT[m.origin]} · <span className="mp-file">{m.file}</span>
            {users && <> · {t("models.local.profile", { names: users })}</>}
          </span>
        </div>
        {!confirm && (
          <div className="mp-actions">
            {!running && (
              <button type="button" className="btn sm" onClick={onUse} disabled={switching || broken} title={broken ? t("models.local.broken.title") : undefined}>
                {t("models.use")}
              </button>
            )}
            {!onOtherDevice && (
              <button type="button" className="btn sm ghost" onClick={() => void reveal()} aria-label={t("models.local.reveal.aria", { name })}>
                <Icon name="folder" small />
                {t("models.local.reveal")}
              </button>
            )}
            {m.deletable && !running && (
              <button type="button" className="btn sm ghost" onClick={() => setConfirm(true)} aria-label={t("models.local.delete.aria", { name })}>
                <Icon name="trash" small />
                {t("models.local.delete")}
              </button>
            )}
          </div>
        )}
      </div>
      {broken && (
        <p className="bad mp-error num">{t("models.local.missing", { n: m.missing_parts.length, parts: m.missing_parts.join(", "), all: String(m.parts) })}</p>
      )}
      {confirm && (
        <div className="mp-confirm" role="group" aria-label={t("models.local.confirm.aria")}>
          <p className="num">
            {m.parts > 1 ? t("models.local.confirm.parts", { name, size: gb(m.size_bytes), n: m.parts }) : t("models.local.confirm", { name, size: gb(m.size_bytes) })}
            {users && ` ${t("models.local.confirm.users", { names: users })}`}
          </p>
          <div className="btn-row">
            <button type="button" className="btn sm danger" onClick={() => void remove()} autoFocus>
              {t("models.local.confirm.yes")}
            </button>
            <button type="button" className="btn sm ghost" onClick={() => setConfirm(false)}>
              {t("models.dl.cancel")}
            </button>
          </div>
        </div>
      )}
      {progress && <SwitchProgress progress={progress} engine={engine} />}
      {error && <p className="bad mp-error">{error}</p>}
    </li>
  );
}

/** The model folders: the app's own (changeable), LM Studio's (read only), further ones. */
export function ModelFolderList({ folders, onChanged }: { folders: ModelFolders | null; onChanged: () => Promise<void> }) {
  const [edit, setEdit] = useState<"app" | "add" | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const id = useId();
  if (!folders) {
    return (
      <div aria-busy="true">
        <span className="bar-ph" />
      </div>
    );
  }
  const extra = folders.extra.map((f) => f.path);
  const save = async (app: string | null, list: string[]) => {
    setSaving(true);
    try {
      await postJson("/models/folders", { app_folder: app, extra: list }, "PUT");
      setError("");
      setEdit(null);
      await onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };
  const appNow = folders.app.default ? null : folders.app.path;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const path = text.trim();
    if (!path) return;
    void (edit === "app" ? save(path, extra) : save(appNow, [...extra, path]));
  };
  const open = (which: "app" | "add") => {
    setEdit(which);
    setText(which === "app" ? folders.app.path : "");
    setError("");
  };
  return (
    <>
      <ul className="mf-list" aria-label={t("models.folders")}>
        <li className="mf-row">
          <div className="mf-text">
            <span className="mp-title">
              <b>{ORIGIN_TEXT.app}</b>
              {folders.app.default && <span className="tag">{t("models.folders.default")}</span>}
            </span>
            <span className="mf-path num">{folders.app.path}</span>
            <span className="help">
              {t("models.folders.app.note")} {folders.app.exists ? "" : t("models.folders.app.later")}
            </span>
          </div>
          {edit !== "app" && (
            <div className="mp-actions">
              <button type="button" className="btn sm ghost" onClick={() => open("app")}>
                <Icon name="pencil" small />
                {t("models.folders.change")}
              </button>
              {!folders.app.default && (
                <button type="button" className="btn sm ghost" disabled={saving} onClick={() => void save(null, extra)}>
                  {t("models.folders.default")}
                </button>
              )}
            </div>
          )}
        </li>
        {folders.lmstudio && (
          <li className="mf-row">
            <div className="mf-text">
              <span className="mp-title">
                <b>LM Studio</b>
                <span className="tag">{t("models.folders.readOnly")}</span>
                {!folders.lmstudio.exists && <span className="tag">{t("models.folders.notFound")}</span>}
              </span>
              <span className="mf-path num">{folders.lmstudio.path}</span>
            </div>
          </li>
        )}
        {folders.extra.map((f) => (
          <li key={f.path} className="mf-row">
            <div className="mf-text">
              <span className="mp-title">
                <b>{ORIGIN_TEXT.extra}</b>
                {!f.exists && <span className="tag">{t("models.folders.notFound")}</span>}
              </span>
              <span className="mf-path num">{f.path}</span>
            </div>
            <div className="mp-actions">
              <button
                type="button"
                className="btn sm ghost"
                disabled={saving}
                onClick={() => void save(appNow, extra.filter((x) => x !== f.path))}
                aria-label={t("models.folders.remove.aria", { path: f.path })}
              >
                <Icon name="close" small />
                {t("models.folders.remove")}
              </button>
            </div>
          </li>
        ))}
      </ul>
      {edit ? (
        <form className="mp-form" aria-label={edit === "app" ? t("models.folders.changeApp") : t("models.folders.add")} onSubmit={submit} onKeyDown={(e) => e.key === "Escape" && setEdit(null)}>
          <label className="field-label" htmlFor={`${id}-folder`}>
            {edit === "app" ? t("models.folders.newApp") : t("models.folders.withModels")} <small>{t("models.folders.fullPath")}</small>
          </label>
          <div className="mp-inline">
            <input id={`${id}-folder`} className="input num" value={text} spellCheck={false} autoFocus onChange={(e) => setText(e.target.value)} />
            <button type="submit" className="btn sm primary" disabled={saving || !text.trim()}>
              {edit === "app" ? t("models.save") : t("models.folders.addShort")}
            </button>
            <button type="button" className="btn sm ghost" onClick={() => setEdit(null)}>
              {t("models.dl.cancel")}
            </button>
          </div>
          {error && <p className="bad">{error}</p>}
        </form>
      ) : (
        <>
          {error && <p className="bad">{error}</p>}
          <div className="btn-row">
            <button type="button" className="btn sm" onClick={() => open("add")} data-spot-focus>
              <Icon name="plus" small />
              {t("models.folders.add")}
            </button>
          </div>
        </>
      )}
    </>
  );
}
