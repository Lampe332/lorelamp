// Model downloads on screen: the panel with every download (progress, speed, pause, resume, cancel),
// the sidebar link "Modelle finden" with the number of running downloads, the phone's badge, and the
// field "Hugging-Face-Link einfügen" with the choice of size tier (Klein / Ausgewogen / Groß).

import { rich } from "../i18n/rich";
import { t, texts } from "../i18n";
import { useId, useState, type FormEvent } from "react";
import { postJson } from "../api";
import { Icon } from "../ui";
import {
  downloadAction,
  dlErrorText,
  errorOf,
  etaText,
  openDownloads,
  size,
  speedText,
  startDownload,
  TIER_NAME,
  TIERS,
  useDownloads,
} from "./store";
import type { Choice, Download, Resolved, Tier } from "./types";
import "./models.css";

const STATE: Record<Download["state"], string> = texts(
  {},
  { queued: "models.dl.state.queued", running: "models.dl.state.running", paused: "models.dl.state.paused", done: "models.dl.state.done", failed: "models.dl.state.failed" },
);

/** The sidebar's way to the model suggestions, with the number of downloads still running. */
export function ModelsNavLink({ current }: { current: boolean }) {
  const { list } = useDownloads();
  const n = openDownloads(list).length;
  return (
    <a href="#/settings/suggestions" aria-current={current ? "page" : undefined} data-log-label="Modelle finden">
      <Icon name="model" />
      {t("shell.nav.modelsFind")}
      {n > 0 && (
        <span className="dl-count num" aria-label={t("shell.downloads.running", { n })}>
          {n}
        </span>
      )}
    </a>
  );
}

/** The phone's top bar: a small badge while downloads run (nothing otherwise). */
export function DownloadsBadge() {
  const { list } = useDownloads();
  const n = openDownloads(list).length;
  if (n === 0) return null;
  return (
    <a className="icon-btn dl-badge" href="#/settings/suggestions" aria-label={t("shell.downloads.running", { n })}>
      <Icon name="download" />
      <span className="dl-count num">{n}</span>
    </a>
  );
}

/** Every download, newest last. `empty`: what to say when there is none (null: show nothing). */
export function DownloadsPanel({ empty }: { empty: string | null }) {
  const { list, folder } = useDownloads();
  if (list === null) {
    return (
      <div aria-busy="true">
        <span className="bar-ph" />
      </div>
    );
  }
  if (list.length === 0) return empty ? <p className="help">{empty}</p> : null;
  return (
    <>
      <ul className="dl-list" aria-label={t("models.dl.title")}>
        {list.map((d) => (
          <DownloadRow key={d.id} d={d} />
        ))}
      </ul>
      {folder && (
        <p className="help dl-folder" title={folder}>
          {t("models.dl.folder", { folder })}
        </p>
      )}
    </>
  );
}

function DownloadRow({ d }: { d: Download }) {
  const [confirm, setConfirm] = useState(false);
  const [error, setError] = useState("");
  const pct = d.total > 0 ? Math.min(100, (d.done / d.total) * 100) : 0;
  const act = async (a: "pause" | "resume" | "remove") => {
    setError("");
    try {
      await downloadAction(d.id, a);
    } catch (e) {
      setError(errorOf(e));
    }
  };
  const parts = d.files.length > 1 ? t("models.dl.files", { n: d.files.length }) : null;
  const eta = d.state === "running" ? etaText(d.total - d.done, d.speed) : null;
  return (
    <li className={`dl-row dl-${d.state}`} aria-label={t("models.dl.row", { name: d.name })}>
      <div className="dl-top">
        <div className="dl-text">
          <b className="dl-name">{d.name}</b>
          <span className="dl-repo">
            {d.repo}
            {parts && ` · ${parts}`}
          </span>
        </div>
        <span className={`dl-state st-${d.state}`}>{STATE[d.state]}</span>
      </div>
      {d.state !== "done" && (
        <div className="dl-bar" role="progressbar" aria-label={t("models.dl.progress", { name: d.name })} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
          <i style={{ width: `${pct}%` }} />
        </div>
      )}
      <div className="dl-foot">
        <span className="dl-figs num">
          {d.state === "done" ? (
            <>{t("models.dl.doneIn", { size: size(d.total) })}</>
          ) : (
            <>
              {t("models.dl.of", { done: size(d.done), total: size(d.total) })}
              {d.state === "running" && d.speed > 0 && ` · ${speedText(d.speed)}`}
              {eta && ` · ${eta}`}
            </>
          )}
        </span>
        {!confirm && (
          <span className="dl-actions">
            {(d.state === "running" || d.state === "queued") && (
              <button type="button" className="btn sm ghost" onClick={() => void act("pause")}>
                {t("models.dl.pause")}
              </button>
            )}
            {(d.state === "paused" || d.state === "failed") && (
              <button type="button" className="btn sm" onClick={() => void act("resume")}>
                {d.state === "failed" ? t("models.dl.retry") : t("models.dl.resume")}
              </button>
            )}
            {d.state === "done" ? (
              <button type="button" className="btn sm ghost" onClick={() => void act("remove")}>
                {t("models.dl.remove")}
              </button>
            ) : (
              <button type="button" className="btn sm ghost" onClick={() => setConfirm(true)} aria-label={t("models.dl.cancel.aria", { name: d.name })}>
                {t("models.dl.cancel")}
              </button>
            )}
          </span>
        )}
      </div>
      {d.state === "failed" && <p className="bad dl-error">{dlErrorText(d.error)}</p>}
      {confirm && (
        <div className="mp-confirm" role="group" aria-label={t("models.dl.confirm.aria")}>
          <p>{t("models.dl.confirm")}</p>
          <div className="btn-row">
            <button type="button" className="btn sm danger" onClick={() => void act("remove")} autoFocus>
              {t("models.dl.confirm.yes")}
            </button>
            <button type="button" className="btn sm ghost" onClick={() => setConfirm(false)}>
              {t("models.dl.confirm.no")}
            </button>
          </div>
        </div>
      )}
      {error && <p className="bad dl-error">{error}</p>}
    </li>
  );
}

/** The choice of a size tier for a resolved repo, then the download. `onStarted` after the start. */
export function TierPicker({ r, onStarted }: { r: Resolved; onStarted?: () => void }) {
  const avail = TIERS.filter((t) => r.groups[t].choices.length > 0);
  const linked = r.file ? TIERS.find((t) => r.groups[t].choices.some((c) => c.files.some((f) => f.path === r.file))) : undefined;
  const [tier, setTier] = useState<Tier | null>(linked ?? (avail.includes("balanced") ? "balanced" : (avail[0] ?? null)));
  const group = tier ? r.groups[tier] : null;
  const linkedKey = r.file && group?.choices.find((c) => c.files.some((f) => f.path === r.file))?.key;
  const [key, setKey] = useState<string | null>(linkedKey ?? group?.recommended ?? null);
  const [proj, setProj] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const id = useId();
  const choice: Choice | undefined = group?.choices.find((c) => c.key === key) ?? group?.choices[0];
  const projFile = r.mmproj[0];
  const pickTier = (t: Tier) => {
    setTier(t);
    setKey(r.groups[t].recommended);
  };
  const start = async () => {
    if (!choice) return;
    setBusy(true);
    setError("");
    try {
      const files = choice.files.map((f) => f.path);
      if (proj && projFile) files.push(projFile.path);
      await startDownload({ repo: r.repo, files });
      onStarted?.();
    } catch (e) {
      setError(errorOf(e));
    } finally {
      setBusy(false);
    }
  };
  if (avail.length === 0) {
    return <p className="help">{t("models.tp.noFile")}</p>;
  }
  return (
    <div className="tp">
      <div className="tp-tiers" role="radiogroup" aria-label={t("models.tp.size")}>
        {TIERS.map((t) => {
          const g = r.groups[t];
          const rec = g.choices.find((c) => c.key === g.recommended);
          return (
            <button
              key={t}
              type="button"
              role="radio"
              aria-checked={tier === t}
              className="tp-tier"
              disabled={g.choices.length === 0}
              onClick={() => pickTier(t)}
            >
              <b>{TIER_NAME[t]}</b>
              <span className="num">{rec ? size(rec.size) : "–"}</span>
            </button>
          );
        })}
      </div>
      {group && group.choices.length > 1 && (
        <label className="field">
          <span className="field-label">{t("models.tp.file")}</span>
          <select className="input" id={id} value={choice?.key ?? ""} onChange={(e) => setKey(e.target.value)}>
            {group.choices.map((c) => (
              <option key={c.key} value={c.key}>
                {c.quant ?? c.key} · {size(c.size)}
                {c.files.length > 1 ? ` · ${t("models.tp.parts", { n: c.files.length })}` : ""}
                {c.key === group.recommended ? ` · ${t("models.tp.recommended")}` : ""}
              </option>
            ))}
          </select>
        </label>
      )}
      {choice && (
        <p className="tp-file num" title={choice.key}>
          {choice.key.split("/").pop()}
          {choice.files.length > 1 && ` · ${t("models.tp.parts", { n: choice.files.length })}`}
        </p>
      )}
      {projFile && (
        <label className="tp-proj">
          <input type="checkbox" checked={proj} onChange={(e) => setProj(e.target.checked)} />
          <span>
            {rich("models.tp.proj", { file: <small className="num">({projFile.path.split("/").pop()}, {size(projFile.size)})</small> })}
          </span>
        </label>
      )}
      <div className="tp-go">
        <button type="button" className="btn primary" disabled={!choice || busy} onClick={() => void start()}>
          <Icon name="download" small />
          {choice ? t("models.tp.get", { size: size(choice.size + (proj && projFile ? projFile.size : 0)) }) : t("models.get")}
        </button>
        {r.license && <span className="help">{t("models.tp.license", { license: r.license })}</span>}
      </div>
      {error && <p className="bad">{error}</p>}
    </div>
  );
}

/** "Hugging-Face-Link einfügen": the repo behind a link, its GGUF files in three tiers, download. */
export function HfLink({ spot = "hf-link" }: { spot?: string }) {
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [r, setR] = useState<Resolved | null>(null);
  const [started, setStarted] = useState(false);
  const id = useId();
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!url.trim()) return;
    setBusy(true);
    setError("");
    setR(null);
    setStarted(false);
    try {
      setR(await postJson<Resolved>("/models/resolve", { url: url.trim() }));
    } catch (err) {
      setError(errorOf(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="hf" data-spot={spot}>
      <form className="mp-inline" onSubmit={(e) => void submit(e)}>
        <label className="sr" htmlFor={id}>
          {t("models.hf.label")}
        </label>
        <input
          id={id}
          className="input"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder={t("models.hf.placeholder")}
          spellCheck={false}
          autoComplete="off"
          inputMode="url"
        />
        <button type="submit" className="btn" disabled={busy || !url.trim()}>
          {busy ? t("models.hf.busy") : t("models.showFiles")}
        </button>
      </form>
      {error && <p className="bad">{error}</p>}
      {r && (
        <div className="hf-result">
          <p className="hf-repo">
            <b>{r.repo}</b>
          </p>
          {started ? (
            <p className="help" role="status">
              {t("models.hf.started")}
            </p>
          ) : (
            <TierPicker key={`${r.repo}/${r.file ?? ""}`} r={r} onStarted={() => setStarted(true)} />
          )}
        </div>
      )}
    </div>
  );
}

/** The row for the tab "Modell": download a model by its Hugging Face link, and the downloads. */
export function ModelDownloadRow() {
  return (
    <div className="set-row">
      <div className="set-label">
        <h2>{t("models.dlRow.title")}</h2>
        <p>
          {rich("models.dlRow.note", {
            link: (
              <a href="#/settings/suggestions" className="link">
                {t("models.sug.title")}
              </a>
            ),
          })}
        </p>
      </div>
      <div className="set-panel">
        <HfLink spot="model-hf-link" />
        <DownloadsPanel empty={null} />
      </div>
    </div>
  );
}
