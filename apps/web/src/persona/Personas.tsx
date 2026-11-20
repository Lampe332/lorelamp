// The management screen: a list beside the editor of one persona.

import { useState } from "react";
import { download, errorText, type Persona } from "../api";
import { t } from "../i18n";
import { useSpot } from "../spot";
import { Icon, Portrait } from "../ui";
import { about, type PersonaSel } from "./common";
import { PersonaEditor, type Draft } from "./PersonaEditor";

type ScreenProps = {
  /** null while the library has not arrived. */
  personas: Persona[] | null;
  activeId: number | null;
  /** From the address: one persona, a new one, or none (the list; a desktop then shows the active one). */
  sel: PersonaSel | null;
  onActivate: (id: number) => void;
  /** Reloads the library after a change (other devices follow over their socket). */
  onChanged: () => Promise<void>;
  onToast: (text: string) => void;
};

export function Personas({ personas, activeId, sel, onActivate, onChanged, onToast }: ScreenProps) {
  // Unsaved text per persona, so a look at another one does not lose it.
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});
  useSpot(`${String(sel)}:${personas !== null}`);
  const shownId: PersonaSel | null = sel ?? activeId;
  const shown = shownId === "new" ? null : (personas?.find((p) => p.id === shownId) ?? null);
  const missing = personas !== null && shownId !== "new" && shownId !== null && !shown;

  const [exporting, setExporting] = useState(false);
  const exportAll = async () => {
    setExporting(true);
    try {
      await download("/personas/export", {}, "personas.zip");
    } catch (e) {
      onToast(t("cards.exportFailed", { error: errorText(e) }));
    } finally {
      setExporting(false);
    }
  };

  const setDraft = (key: string, d: Draft | null) =>
    setDrafts((all) => {
      const { [key]: _old, ...rest } = all;
      return d ? { ...rest, [key]: d } : rest;
    });

  return (
    <div className="chat-main">
      <header className="char-head settings-head persona-head">
        <div className="chat-head">
          {sel !== null && (
            <a className="icon-btn back" href="#/personas" aria-label={t("cards.persona.list.back")}>
              <Icon name="back" />
            </a>
          )}
          <div className="t">
            <h1>{t("cards.persona.list.title")}</h1>
            <span>{t("cards.persona.list.text")}</span>
          </div>
          <button
            type="button"
            className="btn sm"
            onClick={() => void exportAll()}
            disabled={exporting || !personas?.length}
            aria-label={t("cards.persona.list.export")}
            title={t("cards.persona.list.export.title")}
            data-spot="personas-export"
            data-spot-focus
          >
            <Icon name="download" small />
            <span className="hide-sm">{t("cards.persona.list.export")}</span>
          </button>
          <a className="btn sm has-dot" href="#/personas/new" aria-label={t("cards.persona.list.new")} aria-current={sel === "new" ? "page" : undefined}>
            <Icon name="plus" small />
            <span className="hide-sm">{t("cards.persona.list.new")}</span>
            {"new" in drafts && <i className="unsaved" title={t("cards.unsaved")} />}
          </a>
        </div>
      </header>
      <div className="view">
        <div className={`page personas${sel === null ? " at-list" : " at-editor"}`}>
          <nav className="pl" aria-label={t("cards.persona.list.all")}>
            {personas === null ? (
              <div className="pl-row" aria-busy="true">
                <span className="bar-ph" />
              </div>
            ) : (
              personas.map((p) => (
                <a
                  key={p.id}
                  className="pl-row"
                  href={`#/personas/${p.id}`}
                  aria-current={p.id === shownId ? "page" : undefined}
                  data-log-label="Persona öffnen"
                >
                  <Portrait who={p} kind="persona" className="av" />
                  <span className="pl-text">
                    <b>{p.name}</b>
                    <small>{about(p)}</small>
                  </span>
                  {String(p.id) in drafts && <i className="pl-dirty" title={t("cards.unsaved")} />}
                  {p.id === activeId && (
                    <span className="tag pl-on">
                      <Icon name="check" small />
                      {t("cards.persona.list.active")}
                    </span>
                  )}
                </a>
              ))
            )}
          </nav>
          <div className="pe-pane">
            {personas === null ? null : missing ? (
              <div className="pe">
                <p className="empty-line">{t("cards.persona.list.gone")}</p>
              </div>
            ) : shownId === "new" || shown ? (
              <PersonaEditor
                key={String(shownId)}
                persona={shown}
                isActive={!!shown && shown.id === activeId}
                isLast={personas.length <= 1}
                draft={drafts[String(shownId)]}
                onDraft={(d) => setDraft(String(shownId), d)}
                onActivate={onActivate}
                onChanged={onChanged}
                onToast={onToast}
              />
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}
