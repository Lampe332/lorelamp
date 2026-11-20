// "Übernehmen" of a SillyTavern installation asks first: the dialog shows every card, persona and chat
// the installation holds (read by the server, nothing imported yet), with what is already in the app
// unticked and what cannot be read shown but not choosable. Only "Importieren" starts the import of
// exactly the choice; its progress and result stay in the dialog until "Fertig".

import { useEffect, useRef, useState } from "react";
import { stContents, stImageUrl, type Library, type StCard, type StChat, type StContents, type StInstall } from "../api";
import { dateFormat, num, raw, t, type Key } from "../i18n";
import { Icon, Portrait } from "../ui";
import { startSillyTavernImport, useImportJob, type Job, type Row } from "./job";
import { reason } from "./reasons";
import "./stpick.css";

type Load = { kind: "loading" } | { kind: "ready"; data: StContents } | { kind: "failed"; code: string };

const day = (seconds: number) => dateFormat({ day: "numeric", month: "short", year: "numeric" }).format(new Date(seconds * 1000));

/** "Theo - 2026-03-01@18h30m00s" from "Theo/Theo - 2026-03-01@18h30m00s.jsonl". */
const chatTitle = (id: string) => (id.split("/").pop() ?? id).replace(/\.jsonl$/i, "");
const cardName = (c: StCard) => c.name ?? c.id;
const noPic = (name: string) => ({ id: 0, name, has_avatar: false, avatar_rev: 0, avatar_pos: [50, 20] as [number, number] });

const toggled = (set: Set<string>, id: string, on: boolean) => {
  const next = new Set(set);
  if (on) next.add(id);
  else next.delete(id);
  return next;
};

/** What is ticked when the dialog opens: everything new that can be read. */
function firstChoice(d: StContents) {
  const fresh = (x: { duplicate_of: number | null; error: string | null }) => x.duplicate_of === null && !x.error;
  const cards = new Set(d.characters.filter(fresh).map((c) => c.id));
  const known = new Set(d.characters.filter((c) => c.duplicate_of !== null).map((c) => c.id));
  const chats = new Set(d.chats.filter((c) => fresh(c) && c.card !== null && (cards.has(c.card) || known.has(c.card))).map((c) => c.id));
  return { cards, personas: new Set(d.personas.filter(fresh).map((p) => p.id)), chats };
}

// ---------------------------------------------------------------------------------------------
// Pieces

function Tile(props: { name: string; sub: string; src?: string; on: boolean; error?: string | null; dup: boolean; onToggle: (on: boolean) => void }) {
  const { name, sub, src, on, error, dup, onToggle } = props;
  const pic = <Portrait who={noPic(name)} className="x6-pic" preview={src} />;
  if (error) {
    return (
      <li className="x6-tile is-error">
        <div className="x6-tile-in">
          {pic}
          <span className="x6-tname">{name}</span>
          <span className="x6-tsub">
            <Icon name="alert" small />
            {reason(error)}
          </span>
        </div>
      </li>
    );
  }
  return (
    <li className={`x6-tile${on ? " on" : ""}${dup ? " is-dup" : ""}`}>
      <label className="x6-tile-in" data-log-label="SillyTavern-Eintrag wählen">
        {pic}
        <span className="check x6-check">
          <input type="checkbox" checked={on} onChange={(e) => onToggle(e.target.checked)} aria-label={t("system.stp.takeOne", { name })} data-log-label="SillyTavern-Eintrag wählen" />
          <Icon name="check" small />
        </span>
        <span className="x6-tname">{name}</span>
        <span className="x6-tsub">
          {dup && <Icon name="copy" small />}
          {sub}
        </span>
      </label>
    </li>
  );
}

function SectionHead(props: { id: string; title: string; chosen: number; of: number; onAll: () => void; onNone: () => void }) {
  const { id, title, chosen, of, onAll, onNone } = props;
  return (
    <div className="x6-sec-head">
      <h3 id={id}>{title}</h3>
      <span className="x6-n num">
        {t("system.stp.chosenOf", { chosen, of })}
      </span>
      <span className="x6-bulk" role="group" aria-label={t("system.stp.choice", { what: title })}>
        <button type="button" className="btn sm ghost" onClick={onAll} aria-label={t("system.stp.allAria", { what: title })}>
          {t("system.stp.all")}
        </button>
        <button type="button" className="btn sm ghost" onClick={onNone} aria-label={t("system.stp.noneAria", { what: title })}>
          {t("system.stp.none")}
        </button>
      </span>
    </div>
  );
}

function Progress({ job }: { job: Job }) {
  const st = job.st ?? { cards: 0, personas: 0, chats: 0, cardsDone: false };
  // Personas go first and send no progress; the cards report theirs; the chats come with the answer.
  const cardsDone = st.cardsDone || (st.cards > 0 && job.live >= 1);
  const done = (job.live > 0 || cardsDone || st.cards === 0 ? st.personas : 0) + (cardsDone ? st.cards : Math.round(job.live * st.cards));
  const share = job.total ? Math.min(1, done / job.total) : 0;
  const line =
    st.cards > 0 && !cardsDone
      ? t("system.stp.prog.cards", { done: Math.round(job.live * st.cards), total: st.cards })
      : st.chats > 0
        ? t("system.stp.prog.chats", { n: st.chats })
        : t("system.stp.prog.saving");
  return (
    <div className="imp-progress x6-progress" role="status">
      <p className="x6-prog-head" id="x6-prog-label">
        {t("system.stp.prog.head", { n: job.total })}
      </p>
      <div className="imp-bar" role="progressbar" aria-labelledby="x6-prog-label" aria-valuemin={0} aria-valuemax={job.total} aria-valuenow={done}>
        <i style={{ transform: `scaleX(${share})` }} />
      </div>
      <p className="imp-count num">
        <b>{line}</b>
      </p>
    </div>
  );
}

type Outcome = "imported" | "duplicate" | "error";
const GROUPS: { kind: Outcome; title: Key; mark: "check" | "copy" | "alert" }[] = [
  { kind: "imported", title: "system.stp.out.imported", mark: "check" },
  { kind: "duplicate", title: "system.stp.out.duplicate", mark: "copy" },
  { kind: "error", title: "system.stp.out.error", mark: "alert" },
];

function Result({ job, data, library }: { job: Job; data: StContents; library: Library | null }) {
  const cards = new Map(data.characters.map((c) => [c.id, c]));
  const personas = new Map(data.personas.map((p) => [p.id, p]));
  const chats = new Map(data.chats.map((c) => [c.id, c]));
  const chars = new Map((library?.characters ?? []).map((c) => [c.id, c.name]));
  const people = new Map((library?.personas ?? []).map((p) => [p.id, p.name]));

  /** Name, what it is and where it now lives in the app. */
  const describe = (r: Row): { name: string; what: string; href?: string } => {
    const [area, ...rest] = r.file.split("/");
    const id = rest.join("/");
    if (area === "chats") {
      const c = chats.get(id);
      const figure = (r.character_id && chars.get(r.character_id)) || (c?.card && cards.get(c.card)?.name) || null;
      return { name: chatTitle(id), what: figure ? t("system.stp.chatWith", { name: figure }) : t("system.stp.chat"), href: r.chat_id ? `#/chat/${r.chat_id}` : undefined };
    }
    if (area === "characters") {
      const c = cards.get(id);
      const name = (r.character_id && chars.get(r.character_id)) || (c ? cardName(c) : id);
      return { name, what: t("system.stp.card"), href: r.character_id ? `#/char/${r.character_id}/settings` : undefined };
    }
    if (r.persona_id || area === "personas" || r.file === "settings.json" || area === "User Avatars") {
      const name = (r.persona_id && people.get(r.persona_id)) || personas.get(id)?.name || t("system.stp.persona");
      return { name, what: t("system.stp.persona"), href: r.persona_id ? `#/personas/${r.persona_id}` : undefined };
    }
    return { name: r.file || "SillyTavern", what: "" };
  };

  const n = (k: Outcome) => job.rows.filter((r) => r.status === k).length;
  return (
    <div className="x6-result">
      <h3 className="x6-result-h" id="x6-result-h" tabIndex={-1}>
        {job.state === "failed" ? t("system.stp.failed") : t("system.stp.done")}
      </h3>
      <dl className="imp-sum num x6-sum">
        {GROUPS.map((g) => (
          <div key={g.kind} className={g.kind === "error" && n("error") ? "has-error" : undefined}>
            <dt>{raw(g.title)}</dt>
            <dd>{num(n(g.kind))}</dd>
          </div>
        ))}
      </dl>
      {GROUPS.filter((g) => n(g.kind) > 0).map((g) => (
        <section key={g.kind} className="x6-out" aria-labelledby={`x6-out-${g.kind}`}>
          <h4 id={`x6-out-${g.kind}`}>{raw(g.title)}</h4>
          <ul className="imp-rows">
            {job.rows.map((r, i) => {
              if (r.status !== g.kind) return null;
              const d = describe(r);
              const hidden = g.kind === "imported" && r.system_skipped ? t("system.stp.hidden", { n: r.system_skipped }) : "";
              const why = g.kind === "duplicate" ? t("system.stp.inApp") : g.kind === "error" ? reason(r.code) : "";
              return (
                <li key={i} className={`imp-row is-${g.kind}`}>
                  <Icon name={g.mark} small />
                  <span className="imp-row-text">
                    {d.href ? (
                      <a className="imp-name" href={d.href}>
                        {d.name}
                      </a>
                    ) : (
                      <b className="imp-name">{d.name}</b>
                    )}
                    <span className="imp-sub">{[d.what, why].filter(Boolean).join(" · ")}</span>
                    {hidden && <span className="x8-hidden">{hidden}</span>}
                  </span>
                </li>
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// The dialog

export function StPicker({ install, title, library, onClose }: { install: StInstall; title: string; library: Library | null; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [cards, setCards] = useState<Set<string>>(() => new Set());
  const [personas, setPersonas] = useState<Set<string>>(() => new Set());
  const [chats, setChats] = useState<Set<string>>(() => new Set());
  const [jobId, setJobId] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const seq = useRef(0);
  const job = useImportJob();
  const mine = jobId !== null && job?.id === jobId ? job : null;
  const running = mine?.state === "running";

  const read = () => {
    const me = ++seq.current;
    setLoad({ kind: "loading" });
    stContents(install).then(
      (data) => {
        if (me !== seq.current) return;
        const first = firstChoice(data);
        setCards(first.cards);
        setPersonas(first.personas);
        setChats(first.chats);
        setLoad({ kind: "ready", data });
      },
      (e: unknown) => me === seq.current && setLoad({ kind: "failed", code: (e as { code?: string }).code ?? "unknown" }),
    );
  };

  useEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
    read();
    return () => {
      seq.current++;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per opening
  }, []);

  // The result takes the focus, so it is read out and Enter does not import twice.
  const ended = mine && !running ? mine.id : null;
  useEffect(() => {
    if (ended) document.getElementById("x6-result-h")?.focus();
  }, [ended]);

  const close = () => !running && onClose();

  const data = load.kind === "ready" ? load.data : null;
  const cardOf = new Map((data?.characters ?? []).map((c) => [c.id, c]));
  /** A chat can be chosen when its character comes along or is already in the app. */
  const allowed = (c: StChat) => !c.error && c.card !== null && (cards.has(c.card) || cardOf.get(c.card)?.duplicate_of != null);
  const pick = data
    ? {
        characters: data.characters.filter((c) => !c.error && cards.has(c.id)).map((c) => c.id),
        personas: data.personas.filter((p) => !p.error && personas.has(p.id)).map((p) => p.id),
        chats: data.chats.filter((c) => allowed(c) && chats.has(c.id)).map((c) => c.id),
      }
    : { characters: [], personas: [], chats: [] };
  const chosen = pick.characters.length + pick.personas.length + pick.chats.length;

  const start = () => {
    if (!data || chosen === 0) return;
    const id = startSillyTavernImport(install, pick);
    if (id) {
      setNote("");
      setJobId(id);
    } else setNote(t("system.import.busy"));
  };

  const summary = [
    pick.characters.length && t("system.st.cards", { n: pick.characters.length }),
    pick.personas.length && t("system.st.personas", { n: pick.personas.length }),
    pick.chats.length && t("system.stp.chats", { n: pick.chats.length }),
  ].filter(Boolean);

  // Chats under the card they belong to (in the order of the cards), the ones without a card last.
  const groups = data
    ? [
        ...data.characters
          .map((c) => ({ key: c.id, card: c as StCard | null, list: data.chats.filter((x) => x.card === c.id) }))
          .filter((g) => g.list.length > 0),
        ...(data.chats.some((x) => x.card === null) ? [{ key: "", card: null, list: data.chats.filter((x) => x.card === null) }] : []),
      ]
    : [];
  const choosableChats = data ? data.chats.filter(allowed) : [];

  const body = () => {
    if (mine) return running ? <Progress job={mine} /> : <Result job={mine} data={data!} library={library} />;
    if (load.kind === "loading")
      return (
        <p className="imp-wait" role="status">
          {t("system.stp.reading")}
        </p>
      );
    if (load.kind === "failed")
      return (
        <div className="x6-failed">
          <p className="imp-link-msg bad" role="alert">
            <Icon name="alert" small />
            <span>{t("system.stp.readFailed", { reason: reason(load.code) })}</span>
          </p>
          <button type="button" className="btn" onClick={read}>
            {t("system.retry")}
          </button>
        </div>
      );
    const d = load.data;
    if (d.characters.length + d.personas.length + d.chats.length === 0) return <p className="x6-empty">{t("system.stp.empty")}</p>;
    return (
      <>
        <div className="x6-picks">
          {d.characters.length > 0 && (
            <section className="x6-sec" aria-labelledby="x6-cards-h">
              <SectionHead
                id="x6-cards-h"
                title={t("system.stp.cards")}
                chosen={pick.characters.length}
                of={d.characters.filter((c) => !c.error).length}
                onAll={() => setCards(new Set(d.characters.filter((c) => !c.error).map((c) => c.id)))}
                onNone={() => setCards(new Set())}
              />
              <ul className="x6-grid">
                {d.characters.map((c) => (
                  <Tile
                    key={c.id}
                    name={cardName(c)}
                    sub={c.duplicate_of !== null ? t("system.stp.inAppCap") : c.chats > 0 ? t("system.stp.chats", { n: c.chats }) : t("system.stp.noChat")}
                    src={c.has_image ? stImageUrl(install, { card: c.id }) : undefined}
                    on={cards.has(c.id)}
                    error={c.error}
                    dup={c.duplicate_of !== null}
                    onToggle={(on) => setCards((s) => toggled(s, c.id, on))}
                  />
                ))}
              </ul>
            </section>
          )}
          {d.personas.length > 0 && (
            <section className="x6-sec" aria-labelledby="x6-personas-h">
              <SectionHead
                id="x6-personas-h"
                title={t("system.stp.personas")}
                chosen={pick.personas.length}
                of={d.personas.filter((p) => !p.error).length}
                onAll={() => setPersonas(new Set(d.personas.filter((p) => !p.error).map((p) => p.id)))}
                onNone={() => setPersonas(new Set())}
              />
              <ul className="x6-grid x6-grid-persona">
                {d.personas.map((p) => (
                  <Tile
                    key={p.id}
                    name={p.name ?? p.id}
                    sub={p.duplicate_of !== null ? t("system.stp.inAppCap") : t("system.stp.persona")}
                    src={p.has_image ? stImageUrl(install, { persona: p.id }) : undefined}
                    on={personas.has(p.id)}
                    error={p.error}
                    dup={p.duplicate_of !== null}
                    onToggle={(on) => setPersonas((s) => toggled(s, p.id, on))}
                  />
                ))}
              </ul>
            </section>
          )}
        </div>
        {d.chats.length > 0 && (
          <section className="x6-sec x6-sec-chats" aria-labelledby="x6-chats-h">
            <SectionHead
              id="x6-chats-h"
              title={t("system.stp.chatsTitle")}
              chosen={pick.chats.length}
              of={choosableChats.length}
              onAll={() => setChats(new Set(choosableChats.map((c) => c.id)))}
              onNone={() => setChats(new Set())}
            />
            {groups.map((g) => {
              const figure = g.card ? cardName(g.card) : t("system.stp.noCard");
              const waits = g.card !== null && !g.card.error && g.card.duplicate_of === null && !cards.has(g.card.id);
              return (
                <div key={g.key || "-"} className="x6-group" role="group" aria-label={g.card ? t("system.stp.chatsWith", { name: figure }) : figure}>
                  <p className="x6-gname">
                    <b>{figure}</b>
                    {g.card?.duplicate_of != null && <span>{t("system.stp.inApp")}</span>}
                    {waits && <span>{t("system.stp.cardFirst")}</span>}
                  </p>
                  <ul className="x6-chats">
                    {g.list.map((c) => {
                      const ok = allowed(c);
                      const on = ok && chats.has(c.id);
                      const sub = c.error
                        ? reason(c.error)
                        : g.card === null
                          ? t("system.stp.noCardFits")
                          : [t("system.stp.messages", { n: c.messages }), c.last_date ? day(c.last_date) : null, c.duplicate_of !== null ? t("system.stp.inApp") : null]
                              .filter(Boolean)
                              .join(" · ");
                      const name = chatTitle(c.id);
                      const blocked = !!c.error || g.card === null;
                      return (
                        <li key={c.id} className={`x6-chat${on ? " on" : ""}${blocked ? " is-error" : !ok ? " is-off" : ""}`}>
                          <label className="x6-chat-in" data-log-label="SillyTavern-Chat wählen">
                            {blocked ? (
                              <span className="x6-nocheck">
                                <Icon name="alert" small />
                              </span>
                            ) : (
                              <span className="check">
                                <input
                                  type="checkbox"
                                  checked={on}
                                  disabled={!ok}
                                  onChange={(e) => setChats((s) => toggled(s, c.id, e.target.checked))}
                                  aria-label={t("system.stp.takeChat", { name })}
                                  data-log-label="SillyTavern-Chat wählen"
                                />
                                <Icon name="check" small />
                              </span>
                            )}
                            <span className="x6-ctext">
                              <span className="x6-ctitle">{name}</span>
                              <span className="x6-csub num">{sub}</span>
                            </span>
                          </label>
                        </li>
                      );
                    })}
                  </ul>
                </div>
              );
            })}
          </section>
        )}
      </>
    );
  };

  const where = install.user ? `${title} · ${t("system.st.user", { user: install.user })}` : title;
  return (
    <dialog
      ref={dialog}
      className="x6-dlg"
      aria-labelledby="x6-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onMouseDown={(e) => e.target === e.currentTarget && close()}
    >
      <div className="x6-frame">
        <header className="x6-head">
          <i className="pm-grab x6-grab" aria-hidden="true" />
          <h2 id="x6-title">{t("system.st.title")}</h2>
          <p className="x6-where">{where}</p>
          {!mine && (
            <p className="x6-lead">
              {t("system.stp.lead")}
            </p>
          )}
        </header>
        <div className={`x6-body${!mine && data && data.chats.length > 0 && data.characters.length + data.personas.length > 0 ? " has-chats" : ""}`}>{body()}</div>
        <footer className="x6-foot">
          {mine ? (
            <>
              <span className="x6-total" />
              <button type="button" className="btn primary" onClick={onClose} disabled={running}>
                {t("system.stp.finish")}
              </button>
            </>
          ) : (
            <>
              <p className="x6-total num" aria-live="polite">
                {note || (data ? (summary.length ? t("system.stp.chosen", { what: summary.join(" · ") }) : t("system.stp.nothing")) : "")}
              </p>
              <button type="button" className="btn ghost" onClick={onClose}>
                {t("system.cancel")}
              </button>
              <button type="button" className="btn primary" onClick={start} disabled={chosen === 0}>
                <Icon name="download" small />
                {t("system.link.import")}
              </button>
            </>
          )}
        </footer>
      </div>
    </dialog>
  );
}
