// "Aus SillyTavern übernehmen": the server looks for SillyTavern installations on its PC when the page
// opens, so the user does not have to find the data folder. "Übernehmen" on a find opens the choice
// of what to take over (StPicker); choosing the folder by hand stays as the way out.

import { useEffect, useRef, useState } from "react";
import { findSillyTavern, type Library, type StFound, type StInstall } from "../api";
import { t } from "../i18n";
import { rich } from "../i18n/rich";
import { onOtherDevice } from "../phone/shell";
import { Icon } from "../ui";
import { StPicker } from "./StPicker";

type Search = { kind: "searching" } | { kind: "done"; found: StFound } | { kind: "failed" } | { kind: "remote" };

/** The search runs once per page visit; the server keeps its result until "Erneut suchen". It looks at
 *  the PC's drives, so only the PC itself may ask (403 `pc_only` from another device): there the folder
 *  is chosen by hand. */
function useSearch() {
  const [search, setSearch] = useState<Search>(onOtherDevice ? { kind: "remote" } : { kind: "searching" });
  const seq = useRef(0);
  const run = (refresh: boolean) => {
    if (onOtherDevice) return;
    const mine = ++seq.current;
    setSearch({ kind: "searching" });
    findSillyTavern(refresh).then(
      (found) => mine === seq.current && setSearch({ kind: "done", found }),
      () => mine === seq.current && setSearch({ kind: "failed" }),
    );
  };
  useEffect(() => {
    run(false);
    return () => {
      seq.current++;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per visit
  }, []);
  return [search, run] as const;
}

/** Each part stays on one line; the parts wrap between each other. */
const keep = (s: string) => s.replace(/ /g, " ");
const what = (i: StInstall) =>
  [i.user && t("system.st.user", { user: i.user }), t("system.st.cards", { n: i.characters }), t("system.st.personas", { n: i.personas })]
    .filter((s): s is string => !!s)
    .map(keep)
    .join(" · ");

/** The installation's own folder name, and the folder it lies in. */
function split(path: string): [string, string] {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut > 0 && cut < path.length - 1 ? [path.slice(cut + 1), path.slice(0, cut)] : [path, ""];
}

export function SillyTavernImport({ running, library, onPickFolder }: { running: boolean; library: Library | null; onPickFolder: () => void }) {
  const [search, run] = useSearch();
  // "Übernehmen" asks first (StPicker): what to take over, then the import of exactly that.
  const [picking, setPicking] = useState<{ install: StInstall; title: string } | null>(null);
  const installs = search.kind === "done" ? search.found.installs : [];
  const found = installs.length > 0;
  const again = (
    <button className="btn ghost imp-st-again" onClick={() => run(true)} disabled={search.kind === "searching"}>
      {t("system.st.again")}
    </button>
  );
  const pick = (
    <button className={`btn ${found ? "ghost" : "solid"}`} onClick={onPickFolder} disabled={running}>
      <Icon name="folder" small />
      {t("system.st.pickFolder")}
    </button>
  );

  return (
    <section className="imp-block imp-st" data-spot="import-sillytavern" aria-labelledby="imp-st-title">
      <h2 id="imp-st-title">{t("system.st.title")}</h2>
      {search.kind === "searching" && (
        <p className="imp-wait" role="status">
          {t("system.st.searching")}
        </p>
      )}
      {search.kind !== "searching" &&
        (found ? (
          <p>{t("system.st.foundLead")}</p>
        ) : (
          <p>
            {search.kind === "failed" ? `${t("system.st.failed")} ` : search.kind === "remote" ? "" : `${t("system.st.none")} `}
            {rich("system.st.how", { folder: <span className="imp-file">data/default-user</span>, characters: <span className="imp-file">characters</span> })}
          </p>
        ))}
      {found && (
        <>
          <div className="imp-st-head">
            <h3 id="imp-st-found">{t("system.st.found")}</h3>
            {again}
          </div>
          <ul className="imp-st-list" aria-labelledby="imp-st-found">
            {installs.map((i, n) => {
              const [name, where] = split(i.path);
              return (
              <li key={`${i.path}|${i.user ?? ""}`} className="imp-st-row">
                <span className="imp-st-text" id={`imp-st-${n}`}>
                  <span className="imp-st-name">{name}</span>
                  {where && <span className="imp-st-where">{t("system.st.in", { where })}</span>}
                  <span className="imp-st-what">{what(i)}</span>
                </span>
                <button className="btn" onClick={() => setPicking({ install: i, title: name })} disabled={running} aria-describedby={`imp-st-${n}`} aria-haspopup="dialog">
                  {t("system.st.take")}
                </button>
              </li>
              );
            })}
          </ul>
        </>
      )}
      {search.kind === "done" && !search.found.complete && (
        <p className="imp-note">{t("system.st.incomplete")}</p>
      )}
      <div className="btn-row imp-st-actions">
        {pick}
        {!found && search.kind !== "searching" && search.kind !== "remote" && again}
      </div>
      {picking && <StPicker install={picking.install} title={picking.title} library={library} onClose={() => setPicking(null)} />}
    </section>
  );
}
