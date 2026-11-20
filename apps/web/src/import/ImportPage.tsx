// The import page (`#/import`): many cards at once from files, folders or a drop, a link, persona
// files and a SillyTavern data folder. The import itself runs in `job.ts`, so it goes on while the
// user looks elsewhere; this page shows its progress and, afterwards, what happened to each file.

import { useEffect, useRef, useState, type DragEvent } from "react";
import { num, type CharInfo, type Library } from "../api";
import { raw, t, type Key } from "../i18n";
import { rich } from "../i18n/rich";
import { useSpot } from "../spot";
import { Icon, type IconName } from "../ui";
import { ImportFit } from "./ImportFit";
import { cancelImport, clearImport, droppedFiles, pickedFiles, planImport, startImport, useImportJob, type Job, type Picked, type Row } from "./job";
import { LinkImport } from "./LinkImport";
import { cardJson, copiedImageAddress, linksIn, takeLinks } from "./links";
import { reason } from "./reasons";
import { SillyTavernImport } from "./SillyTavernImport";

/** Files that hold no card at all (notes, pictures without card data): they go in a folded group. */
const SKIP = new Set(["unknown_format", "no_card", "no_persona"]);
type Kind = "imported" | "duplicate" | "error" | "skipped";
const kindOf = (r: Row): Kind => (r.status === "error" && r.code && SKIP.has(r.code) ? "skipped" : r.status);

const FILTERS: { id: Exclude<Kind, "skipped"> | "all"; label: Key }[] = [
  { id: "all", label: "system.import.filter.all" },
  { id: "imported", label: "system.import.imported" },
  { id: "duplicate", label: "system.import.duplicate" },
  { id: "error", label: "system.import.error" },
];

const MARK: Record<Kind, IconName> = { imported: "check", duplicate: "copy", error: "alert", skipped: "close" };

/** What the import reads, and where such files come from. */
const FORMATS: [string, Key][] = [
  ["png", "system.import.format.png"],
  ["JSON", "system.import.format.json"],
  ["CHARX", "system.import.format.charx"],
  ["BYAF", "system.import.format.byaf"],
  ["Perchance", "system.import.format.perchance"],
  ["YAML", "system.import.format.yaml"],
  ["ZIP", "system.import.format.zip"],
];

// ---------------------------------------------------------------------------------------------
// Progress and result

function Progress({ job }: { job: Job }) {
  const done = Math.min(job.total, job.sent + Math.round(job.live * job.current));
  const share = job.total ? done / job.total : 0;
  const imported = job.rows.filter((r) => r.status === "imported").length;
  const dupes = job.rows.filter((r) => r.status === "duplicate").length;
  return (
    <div className="imp-progress">
      <p className="imp-head" id="imp-progress-label">
        {job.st ? t("system.import.progress.st", { n: job.total }) : t("system.import.progress.files", { n: job.total })}
      </p>
      <div className="imp-bar" role="progressbar" aria-labelledby="imp-progress-label" aria-valuemin={0} aria-valuemax={job.total} aria-valuenow={done}>
        <i style={{ transform: `scaleX(${share})` }} />
      </div>
      <p className="imp-count num">
        <b>
          {t("system.import.ofTotal", { done, total: job.total })}
        </b>
        {job.batches > 1 && (
          <span>
            {t("system.import.package", { batch: job.batch, batches: job.batches })}
          </span>
        )}
        {(imported > 0 || dupes > 0) && (
          <span>
            {t("system.import.importedN", { n: imported })}
            {dupes > 0 && ` · ${t("system.import.duplicateN", { n: dupes })}`}
          </span>
        )}
      </p>
      {job.batches > 1 && (
        <button className="btn ghost" onClick={cancelImport} disabled={job.stopping}>
          {job.stopping ? t("system.import.stopping") : t("system.cancel")}
        </button>
      )}
    </div>
  );
}

function stateLine(job: Job): string {
  const files = job.st ? t("system.import.entries", { n: job.total }) : t("system.import.files", { n: job.total });
  if (job.state === "cancelled") return t("system.import.state.cancelled", { sent: job.sent, files });
  if (job.state === "failed") return t("system.import.state.failed", { sent: job.sent, files });
  return t("system.import.state.done", { files });
}

function Results({ job, library }: { job: Job; library: Library | null }) {
  const [filter, setFilter] = useState<(typeof FILTERS)[number]["id"]>("all");
  const rows = job.rows.map((r, i) => ({ r, i, kind: kindOf(r) }));
  const n = (k: Kind) => rows.filter((x) => x.kind === k).length;
  const counts = { all: rows.length - n("skipped"), imported: n("imported"), duplicate: n("duplicate"), error: n("error") };
  // A filter whose rows are gone falls back to all.
  const shownFilter = filter !== "all" && counts[filter] === 0 ? "all" : filter;
  const listed = rows.filter((x) => x.kind !== "skipped" && (shownFilter === "all" || x.kind === shownFilter));
  const skipped = rows.filter((x) => x.kind === "skipped");
  const chars = new Map((library?.characters ?? []).map((c) => [c.id, c]));
  const personas = new Map((library?.personas ?? []).map((p) => [p.id, p]));
  const newCards = rows.flatMap((x) => (x.r.status === "imported" && x.r.character_id && !x.r.chat_id ? [chars.get(x.r.character_id)] : [])).filter((c): c is CharInfo => !!c);

  const target = (r: Row) => {
    // A chat taken over from SillyTavern ("chats/<folder>/<file>.jsonl"): its title, opening the chat.
    if (r.chat_id) return { name: (r.file.split("/").pop() ?? r.file).replace(/\.jsonl$/i, ""), href: `#/chat/${r.chat_id}` };
    if (r.character_id) {
      const c = chars.get(r.character_id);
      return c ? { name: c.name, href: `#/char/${c.id}/settings` } : null;
    }
    if (r.persona_id) {
      const p = personas.get(r.persona_id);
      return p ? { name: p.name, href: `#/personas/${p.id}` } : null;
    }
    return null;
  };

  return (
    <section className="imp-result" aria-labelledby="imp-result-title">
      <div className="imp-result-head">
        <h2 className="sec" id="imp-result-title" tabIndex={-1}>
          {t("system.import.result")}
        </h2>
        <span className="muted">{stateLine(job)}</span>
        <button className="btn sm ghost" onClick={clearImport}>
          {t("system.import.clear")}
        </button>
      </div>
      <dl className="imp-sum num">
        <div>
          <dt>{t("system.import.imported")}</dt>
          <dd>{num(counts.imported)}</dd>
        </div>
        <div>
          <dt>{t("system.import.duplicate")}</dt>
          <dd>{num(counts.duplicate)}</dd>
        </div>
        <div className={counts.error ? "has-error" : undefined}>
          <dt>{t("system.import.error")}</dt>
          <dd>{num(counts.error)}</dd>
        </div>
      </dl>
      {newCards.length > 0 && <ImportFit imported={newCards} library={library} />}

      {counts.all > 0 && (
        <div className="imp-filter" role="group" aria-label={t("system.import.filter")}>
          {FILTERS.filter((f) => f.id === "all" || counts[f.id] > 0).map((f) => (
            <button key={f.id} className="chip" aria-pressed={shownFilter === f.id} onClick={() => setFilter(f.id)}>
              {raw(f.label)} <span className="num">{num(counts[f.id])}</span>
            </button>
          ))}
        </div>
      )}
      {listed.length > 0 && (
        <ul className="imp-rows" aria-label={t("system.import.filesLabel")}>
          {listed.map(({ r, i, kind }) => {
            const to = target(r);
            return (
              <li key={i} className={`imp-row is-${kind}`}>
                <Icon name={MARK[kind]} small />
                <span className="imp-row-text">
                  {to ? (
                    <a href={to.href} className="imp-name">
                      {to.name}
                    </a>
                  ) : (
                    <b className="imp-name">{r.file || t("system.import.file")}</b>
                  )}
                  <span className="imp-sub">
                    {kind === "imported" ? (to ? r.file : `${r.file} · ${t("system.import.row.imported")}`) : kind === "duplicate" ? `${to ? `${r.file} · ` : ""}${t("system.import.row.duplicate")}` : reason(r.code)}
                  </span>
                </span>
                <span className="imp-state">{kind === "imported" ? t("system.import.imported") : kind === "duplicate" ? t("system.import.duplicate") : t("system.import.error")}</span>
              </li>
            );
          })}
        </ul>
      )}
      {skipped.length > 0 && (
        <details className="imp-skip">
          <summary>
            <Icon name="chevron" small />
            {t("system.import.skipped", { n: skipped.length })}
          </summary>
          <p className="imp-note">{t("system.import.skipped.note")}</p>
          <ul className="imp-rows">
            {skipped.map(({ r, i }) => (
              <li key={i} className="imp-row is-skipped">
                <Icon name="close" small />
                <span className="imp-row-text">
                  <b className="imp-name">{r.file || t("system.import.file")}</b>
                  <span className="imp-sub">{reason(r.code)}</span>
                </span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------------------------
// The page

/** Folder pickers need an attribute React does not know. */
const folderInput = (ref: { current: HTMLInputElement | null }) => (el: HTMLInputElement | null) => {
  el?.setAttribute("webkitdirectory", "");
  ref.current = el;
};

export function ImportPage({ library }: { library: Library | null }) {
  useSpot("import");
  const job = useImportJob();
  const running = job?.state === "running";
  const [over, setOver] = useState(false);
  const [note, setNote] = useState("");
  const [touch] = useState(() => matchMedia("(pointer: coarse)").matches);
  const files = useRef<HTMLInputElement>(null);
  const folder = useRef<HTMLInputElement>(null);
  const personaFiles = useRef<HTMLInputElement>(null);
  const stFolder = useRef<HTMLInputElement>(null);
  const depth = useRef(0);
  // An import that began with pasted pictures only: when none of them holds a card, the page says why.
  const pastedPictures = useRef(false);

  const begin = (picked: Picked[], personasOnly = false) => {
    if (picked.length === 0) return;
    pastedPictures.current = false;
    setNote(startImport(picked, { personasOnly }) ? "" : running ? t("system.import.busy") : t("system.import.nothing"));
  };

  /** A card copied as JSON text comes in like a file. */
  const beginText = (text: string) => {
    const name = t("system.import.pastedCard");
    begin([{ file: new File([text.trim()], name, { type: "application/json" }), path: name }]);
  };

  const takeSillyTavern = (picked: Picked[]) => {
    if (picked.length === 0) return;
    if (!planImport(picked).sillyTavern) {
      setNote(t("system.import.notSt"));
      return;
    }
    begin(picked);
  };

  // When an import ends while the page is open, its result comes into view.
  const ended = job?.state !== "running" ? job?.id : undefined;
  const seen = useRef(ended);
  useEffect(() => {
    if (!ended || seen.current === ended) return;
    seen.current = ended;
    const head = document.getElementById("imp-result-title");
    head?.scrollIntoView({ block: "start", behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
    head?.focus({ preventScroll: true });
    // A picture copied in a browser or an image program is only its pixels: the card data that a
    // card PNG carries is lost on the way to the clipboard.
    if (pastedPictures.current && job && job.rows.length > 0 && job.rows.every((r) => r.status === "error" && r.code === "no_card")) {
      setNote(t("system.import.pastedPicture"));
    }
    pastedPictures.current = false;
  }, [ended, job]);

  // Ctrl+V anywhere on the page (not inside a field): files and pictures are imported, links show
  // their previews (see LinkImport), a card copied as JSON text comes in like a file.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if ((e.target as Element | null)?.closest?.("input, textarea, [contenteditable]")) return;
      const data = e.clipboardData;
      if (!data) return;
      const pasted = [...data.files];
      const text = data.getData("text/plain").trim();
      if (pasted.length) {
        e.preventDefault();
        const pictures = pasted.every((f) => f.type.startsWith("image/"));
        // "Bild kopieren" on a web page: the picture's address still leads to the file with the card.
        const address = pictures ? copiedImageAddress(data.getData("text/html")) : null;
        if (address) {
          setNote(takeLinks([address]));
          return;
        }
        begin(pasted.map((file) => ({ file, path: file.name })));
        pastedPictures.current = pictures && !running;
        return;
      }
      if (!text) return;
      e.preventDefault();
      if (cardJson(text)) {
        beginText(text);
        return;
      }
      const links = linksIn(text);
      setNote(links.length ? takeLinks(links) : t("system.import.pastedNothing"));
    };
    document.addEventListener("paste", onPaste);
    return () => document.removeEventListener("paste", onPaste);
  });

  const hasFiles = (e: DragEvent) => e.dataTransfer.types.includes("Files");
  /** Links dragged over from a browser (a link, a picture of a page, the address bar). */
  const hasLinks = (e: DragEvent) => e.dataTransfer.types.includes("text/uri-list");
  const drop = (e: DragEvent) => {
    e.preventDefault();
    depth.current = 0;
    setOver(false);
    if (!hasFiles(e)) {
      const links = linksIn(e.dataTransfer.getData("text/uri-list").replace(/^#.*$/gm, ""));
      if (links.length) setNote(takeLinks(links));
      return;
    }
    if (running) {
      setNote(t("system.import.busy"));
      return;
    }
    void droppedFiles(e.dataTransfer).then((picked) => begin(picked));
  };
  const dragOver = (e: DragEvent) => {
    if (hasFiles(e) || hasLinks(e)) e.preventDefault();
  };
  const dragEnter = (e: DragEvent) => {
    if (!hasFiles(e) && !hasLinks(e)) return;
    depth.current += 1;
    setOver(true);
  };
  const dragLeave = () => {
    depth.current = Math.max(0, depth.current - 1);
    if (depth.current === 0) setOver(false);
  };

  const onPick = (fn: (p: Picked[]) => void) => (e: { target: HTMLInputElement }) => {
    const picked = pickedFiles(e.target.files);
    e.target.value = "";
    fn(picked);
  };

  return (
    <div className="view" onDragOver={dragOver} onDragEnter={dragEnter} onDragLeave={dragLeave} onDrop={drop}>
      <div className="page imp-page">
        <div className="title-row">
          <h1>{t("system.import.title")}</h1>
          <span className="sub hide-sm">{t("system.import.sub")}</span>
          <div className="actions">
            <a className="btn ghost" href="#/chars">
              <Icon name="back" small />
              {t("system.import.toChars")}
            </a>
          </div>
        </div>

        <div className="imp-grid">
          <div className="imp-main">
            <div className={`imp-drop ${over && !running ? "over" : ""} ${running ? "busy" : ""}`} data-spot="import-files">
              {running && job ? (
                <Progress job={job} />
              ) : (
                <>
                  <div className="imp-drop-text">
                    <Icon name="upload" />
                    <p className="imp-head">{touch ? t("system.import.drop.touch") : over ? t("system.import.drop.over") : t("system.import.drop.head")}</p>
                    <p className="imp-lead">
                      {touch
                        ? t("system.import.drop.leadTouch")
                        : t("system.import.drop.lead")}
                    </p>
                    <div className="btn-row">
                      <button className="btn primary" onClick={() => files.current?.click()}>
                        <Icon name="card" small />
                        {t("system.import.pickFiles")}
                      </button>
                      <button className="btn solid" onClick={() => folder.current?.click()}>
                        <Icon name="folder" small />
                        {t("system.import.pickFolder")}
                      </button>
                    </div>
                  </div>
                  <dl className="imp-formats" aria-label={t("system.import.formats")}>
                    {FORMATS.map(([what, from]) => (
                      <div key={what}>
                        <dt>{what === "png" ? t("system.import.format.pngName") : what}</dt>
                        <dd>{raw(from)}</dd>
                      </div>
                    ))}
                  </dl>
                </>
              )}
            </div>
            {note && (
              <p className="imp-link-msg bad" role="alert">
                <Icon name="alert" small />
                <span>{note}</span>
              </p>
            )}
            <LinkImport library={library} touch={touch} onPickFile={() => files.current?.click()} onCardText={beginText} />
          </div>

          <aside className="imp-side" aria-label={t("system.import.more")}>
            <section className="imp-block" data-spot="import-personas" aria-labelledby="imp-personas-title">
              <h2 id="imp-personas-title">{t("system.import.personas")}</h2>
              <p>
                {rich("system.import.personas.how", {
                  backup: <span className="imp-file">personas_….json</span>,
                  settings: <span className="imp-file">settings.json</span>,
                  avatars: <span className="imp-file">User Avatars</span>,
                })}
              </p>
              <button className="btn solid" onClick={() => personaFiles.current?.click()} disabled={running}>
                <Icon name="persona" small />
                {t("system.import.personas.pick")}
              </button>
            </section>
            <SillyTavernImport running={running} library={library} onPickFolder={() => stFolder.current?.click()} />
          </aside>
        </div>

        {job && job.state !== "running" && <Results key={job.id} job={job} library={library} />}

        <input ref={files} type="file" multiple hidden onChange={onPick((p) => begin(p))} />
        <input ref={folderInput(folder)} type="file" multiple hidden onChange={onPick((p) => begin(p))} />
        <input ref={personaFiles} type="file" multiple hidden accept=".json,.png,.zip,application/json,image/png,application/zip" onChange={onPick((p) => begin(p, true))} />
        <input ref={folderInput(stFolder)} type="file" multiple hidden onChange={onPick(takeSillyTavern)} />
      </div>
    </div>
  );
}
