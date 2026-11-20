// Shared pieces of the five start layouts: the data they show, the backdrop, the rows, tiles and
// measurements more than one layout uses.

import { t, texts } from "../i18n";
import { rich } from "../i18n/rich";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { chatName, modelFacts, modelName, num, snippet, when, type CharInfo, type EngineInfo, type Library as Lib, type Recent, type Timings } from "../api";
import type { Command } from "../commands";
import type { HomeLayoutId } from "../theme";
import { hueStyle, Icon, Portrait, type IconName } from "../ui";
import { ModelAct, modelState, NewChat, Star } from "./extras";

/** Planned places of the app (from the plan), named once at the start page's foot until they exist.
 *  Never links, and never in the navigation. */
export const SOON: { id: string; icon: IconName; name: string; note: string }[] = [
  texts({ id: "group", icon: "group" }, { name: "shell.soon.group", note: "shell.soon.group.note" }),
  texts({ id: "lore", icon: "book" }, { name: "shell.soon.lore", note: "shell.soon.lore.note" }),
  texts({ id: "stats", icon: "chart" }, { name: "shell.soon.stats", note: "shell.soon.stats.note" }),
  texts({ id: "memory", icon: "memory" }, { name: "shell.soon.memory", note: "shell.soon.memory.note" }),
];

/** `info`: what `/engine` says about the model (name, quant, context), as the sidebar shows it. */
export type EngineView = { status: "ready" | "loading" | "error" | "offline" | "connecting" | "idle"; label: string; info?: EngineInfo | null };

export type HubProps = {
  library: Lib | null;
  loadError: string;
  /** Why the model is not running, if the app knows. */
  engineNote: string;
  onRetry: () => void;
  layout: HomeLayoutId;
  engine: EngineView;
  /** The newest answer measured since this window opened; newer than the library's. */
  live: Timings | null;
  commands: Command[];
  onPalette: (query?: string) => void;
  onImport: () => void;
  onNewChat: (characterId: number) => void;
};

export type Data = {
  characters: CharInfo[];
  recent: Recent[];
  lead: Recent | undefined;
  /** Favourites first, then the others; each group in the order it was last used. */
  byUse: CharInfo[];
  /** Imported within the last week and not played yet, newest first. */
  fresh: CharInfo[];
  chats: number;
  messages: number;
  timings: Timings | null;
};

const FRESH_DAYS = 7;
const FRESH_MAX = 4;

export function derive(library: Lib, live: Timings | null): Data {
  const { characters, recent, stats } = library;
  const used = recent.map((r) => r.chat.character_id);
  const rank = (c: CharInfo) => {
    const i = used.indexOf(c.id);
    return (c.favorite ? 0 : 1000) + (i < 0 ? 500 : i);
  };
  const byUse = [...characters].sort((a, b) => rank(a) - rank(b));
  const since = Date.now() / 1000 - FRESH_DAYS * 86400;
  // Before the first chat the first steps offer every figure; "new" starts to mean something after it.
  const fresh = (recent.length === 0 ? [] : characters)
    .filter((c) => c.chats === 0 && c.created_at >= since)
    .sort((a, b) => b.created_at - a.created_at || b.id - a.id)
    .slice(0, FRESH_MAX);
  return {
    characters,
    recent,
    lead: recent[0],
    byUse,
    fresh,
    chats: characters.reduce((n, c) => n + c.chats, 0),
    messages: stats?.messages ?? 0,
    timings: newestTimings(live, library),
  };
}

/** The newest answer's measurements: the one measured since this window opened, else the newest the
 *  server keeps (so a reload does not empty them). Start screen and sidebar show the same. */
export const newestTimings = (live: Timings | null, library: Lib | null): Timings | null => live ?? library?.stats?.last_timings ?? null;

export type ViewProps = HubProps & { data: Data; notes: ReactNode };

export const who = (r: Recent) => ({ ...r, id: r.chat.character_id });

/** The server sends the newest message cut after 240 characters. A cut text ends at its last full
 *  sentence when one closes late enough, otherwise after its last whole word with "…". */
const SERVER_CUT = 240;
function excerpt(raw: string): string {
  const text = snippet(raw);
  if ([...raw].length < SERVER_CUT) return text;
  let end = -1;
  for (const m of text.matchAll(/[.!?…]["'»«“”’)]*(?=\s)/g)) end = m.index + m[0].length;
  if (end >= text.length * 0.55) return text.slice(0, end);
  const space = text.lastIndexOf(" ");
  return `${(space > 0 ? text.slice(0, space) : text).replace(/[\s,;:–-]+$/, "")} …`;
}
export const quote = (r: Recent, none: string) => excerpt(r.chat.last) || none;

/** The start screen's backdrop: the last character's picture, blurred, and two slow lights in its
 *  hue, fading into the ground. Without a picture the lights alone carry the character's colour. */
export function Aura({ lead, fallback }: { lead: Recent | undefined; fallback: CharInfo | undefined }) {
  const face = lead ? who(lead) : fallback;
  return (
    <div className="aura" style={hueStyle(face?.name ?? "")} aria-hidden="true">
      {face && <Portrait who={face} className="aura-img" />}
      <i className="aura-light a1" />
      <i className="aura-light a2" />
      <i className="aura-grain" />
    </div>
  );
}

/** A horizontal row that scrolls without a scrollbar: arrow buttons page through it where a pointer
 *  hovers, a finger swipes it on touch screens. The arrows show only while there is more that way. */
export function Shelf({ className = "", list, label, children }: { className?: string; list?: boolean; label: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement & HTMLUListElement>(null);
  const [more, setMore] = useState({ start: false, end: false });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => {
      const start = el.scrollLeft > 2;
      const end = el.scrollLeft + el.clientWidth < el.scrollWidth - 2;
      setMore((m) => (m.start === start && m.end === end ? m : { start, end }));
    };
    update();
    el.addEventListener("scroll", update, { passive: true });
    const seen = new ResizeObserver(update);
    seen.observe(el);
    for (const child of el.children) seen.observe(child);
    return () => {
      el.removeEventListener("scroll", update);
      seen.disconnect();
    };
  }, [children]);
  const page = (dir: 1 | -1) => {
    const el = ref.current;
    if (!el) return;
    const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
    el.scrollBy({ left: dir * Math.max(240, el.clientWidth * 0.8), behavior: calm ? "auto" : "smooth" });
  };
  const Tag = list ? "ul" : "div";
  return (
    <div className={`shelf-wrap${more.start ? " more-start" : ""}${more.end ? " more-end" : ""}`}>
      <Tag ref={ref} className={`shelf ${className}`} aria-label={label}>
        {children}
      </Tag>
      {more.start && (
        <button type="button" className="shelf-arrow prev" onClick={() => page(-1)} aria-label={t("system.hub.shelf.prev", { label })} tabIndex={-1}>
          <Icon name="back" small />
        </button>
      )}
      {more.end && (
        <button type="button" className="shelf-arrow next" onClick={() => page(1)} aria-label={t("system.hub.shelf.next", { label })} tabIndex={-1}>
          <Icon name="back" small />
        </button>
      )}
    </div>
  );
}

/** How many chats a figure has, as its cards and rows say it. */
export const chatsOf = (c: CharInfo) => (c.chats === 0 ? t("system.hub.noChat") : t("system.hub.chats", { n: c.chats }));

/** A figure's first four tags. */
export function FigTags({ c }: { c: CharInfo }) {
  if (c.tags.length === 0) return null;
  return (
    <span className="fig-tags">
      {c.tags.slice(0, 4).map((t) => (
        <span className="tag" key={t}>
          {t}
        </span>
      ))}
    </span>
  );
}

/** A character with room to spare: its poster beside its name, short description and tags; its
 *  star and a new chat sit on the card. */
export function FigureCard({ c }: { c: CharInfo }) {
  return (
    <article className="hx-fig" style={hueStyle(c.name)}>
      <a className="fig-card" href={`#/char/${c.id}/chats`} data-log-label="Figur öffnen">
        <span className="fig-pic">
          <Portrait who={c} className="art" />
        </span>
        <span className="fig-text">
          <b>{c.name}</b>
          <span className="meta num">{chatsOf(c)}</span>
          {c.blurb && <span className="fig-blurb">{c.blurb}</span>}
          <FigTags c={c} />
        </span>
      </a>
      <Star c={c} />
      <NewChat c={c} />
    </article>
  );
}

/** Import as a quiet tile beside the figures: what it does in one line, not a poster-sized void. */
export function AddCard({ onImport }: { onImport: () => void }) {
  return (
    <button type="button" className="add-card" onClick={onImport} data-log-label="Karte importieren">
      <span className="add-ic">
        <Icon name="upload" small />
      </span>
      <span className="add-text">
        <b>{t("system.hub.import")}</b>
        <span>{t("system.hub.import.note")}</span>
      </span>
    </button>
  );
}

export const FEW = 2; // up to this many figures show as cards with their description (more where the column is wide)

/** The figures of a start layout: few as cards with their description, more as posters in a grid or
 *  a scrolling row; the import tile closes the set. */
export function Figures(props: { list: CharInfo[]; onImport: () => void; shelf?: boolean; add?: boolean; few?: number }) {
  const { list, onImport, shelf, add = true, few = FEW } = props;
  const tail = add ? <AddCard key="add" onImport={onImport} /> : null;
  if (list.length <= few) {
    return (
      <div className="figs few">
        {list.map((c) => (
          <FigureCard key={c.id} c={c} />
        ))}
        {tail}
      </div>
    );
  }
  const posters = list.map((c) => <Poster key={c.id} c={c} />);
  return shelf ? (
    <Shelf className="posters" label={t("system.hub.yourFigures")}>
      {posters}
      {tail}
    </Shelf>
  ) : (
    <div className="figs many">
      {posters}
      {tail}
    </div>
  );
}

export function Poster({ c }: { c: CharInfo }) {
  return (
    <article className={`cc hx-cc${c.favorite ? " fav" : ""}`} style={hueStyle(c.name)}>
      <a className="cc-open" href={`#/char/${c.id}/chats`} data-log-label="Figur öffnen">
        <Portrait who={c} className="art" />
        <span className="body">
          <b>{c.name}</b>
          <span className="meta num">{chatsOf(c)}</span>
        </span>
      </a>
      <Star c={c} />
      <NewChat c={c} />
    </article>
  );
}

export const cachePct = (t: Timings) => (t.cached + t.processed > 0 ? Math.round((100 * t.cached) / (t.cached + t.processed)) : null);

/** The search widget at the top of a page: opens the quick actions. */
export function SearchWidget({ onPalette }: { onPalette: () => void }) {
  return (
    <button type="button" className="search-widget" onClick={() => onPalette()} aria-label={t("shell.nav.searchLabelKey")} aria-keyshortcuts="Control+K">
      <Icon name="search" small />
      <span>{t("system.hub.searchDots")}</span>
      <kbd>{t("shell.key.ctrlK")}</kbd>
    </button>
  );
}

/** The quick actions as one row of buttons. */
export function QuickBar({ data, onPalette, onImport }: Pick<ViewProps, "onPalette" | "onImport"> & { data: Data }) {
  return (
    <nav className="quick" aria-label={t("system.hub.quick")}>
      <button type="button" className="btn sm solid" onClick={() => onPalette(t("cmd.newWith", { name: "" }))} disabled={data.characters.length === 0}>
        <Icon name="plus" small />
        {t("system.hub.newChat")}
      </button>
      <button type="button" className="btn sm solid" onClick={onImport}>
        <Icon name="upload" small />
        {t("system.hub.import")}
      </button>
      <a className="btn sm solid" href="#/personas">
        <Icon name="persona" small />
        {t("system.hub.personas")}
      </a>
    </nav>
  );
}

/** "Weiterschreiben": the primary button with a small hop of its arrow on hover. */
export function GoOn() {
  return (
    <span className="btn primary joy">
      {t("system.hub.goOn")}
      <Icon name="next" small />
    </span>
  );
}

export function EngineFigure({ engine }: { engine: EngineView }) {
  return (
    <span className="engine-fig">
      <i className={`dot ${engine.status === "connecting" ? "offline" : engine.status}`} />
      {engine.label}
    </span>
  );
}

export const speed = (t: Timings | null) => (t ? `${num(Math.round(t.tok_s))}` : "–");
const tokens = (n: number) => (
  <>
    {num(n)} <small>{t("system.hub.tokenUnit", { n })}</small>
  </>
);

/** The model's state and the newest answer's measurements (B's tile, D's panel). */
export function EngineStats({ engine, timings, heading }: { engine: EngineView; timings: Timings | null; heading?: boolean }) {
  const pct = timings && cachePct(timings);
  const Label = heading ? "h2" : "span";
  const m = engine.info?.model;
  const name = modelName(m);
  const facts = modelFacts(m);
  const state = modelState(engine);
  return (
    <>
      <Label className="tile-label">
        {t("system.hub.model")}
        <EngineFigure engine={engine} />
      </Label>
      {name && (
        <a className="tile-model" href="#/settings/model" title={m?.file ?? undefined} data-log-label="Modell öffnen">
          <b>{name}</b>
          {facts && <span className="num">{facts}</span>}
        </a>
      )}
      {state.kind !== "ready" && state.kind !== "connecting" && state.kind !== "loading" && (
        <span className="hx-model-need">
          <span>{state.line}</span>
          {state.kind === "idle" && <ModelAct state={state} />}
        </span>
      )}
      <span className="tile-big num">
        {speed(timings)}
        <small>{timings ? t("system.hub.tokSLast") : t("system.hub.notMeasured")}</small>
      </span>
      {timings ? (
        <>
          {pct !== null && (
            <span className="cache-meter num">
              <span className="cache-bar" aria-hidden="true">
                <i style={{ width: `${pct}%` }} />
              </span>
              {t("system.hub.fromCache", { pct: t("system.pct", { n: pct }) })}
            </span>
          )}
          <dl className="tile-facts num" aria-label={t("system.hub.lastAnswer")}>
            <div>
              <dt>{t("system.hub.written")}</dt>
              <dd>
                {tokens(timings.predicted)}
              </dd>
            </div>
            <div>
              <dt>{t("system.hub.read")}</dt>
              <dd>
                {tokens(timings.processed)}
              </dd>
            </div>
            <div>
              <dt>{t("system.hub.cached")}</dt>
              <dd>
                {tokens(timings.cached)}
              </dd>
            </div>
          </dl>
        </>
      ) : (
        <p className="help">{t("system.hub.measureLater")}</p>
      )}
    </>
  );
}

/** A chat as one row with what tells chats of the same figure apart: its name or start, its last
 *  lines, its length and when it was last played (B and D). */
export function ChatLine({ r }: { r: Recent }) {
  return (
    <a className="chat-line" href={`#/chat/${r.chat.id}`} style={hueStyle(r.name)} data-log-label="Chat öffnen">
      <Portrait who={who(r)} className="av" />
      <span className="cl-text">
        <span className="cl-head">
          <b>{r.name}</b>
          <span>{chatName(r.chat)}</span>
        </span>
        <span className="cl-quote">{quote(r, t("system.hub.noMessage"))}</span>
      </span>
      <span className="cl-meta num">
        {t("system.hub.messages", { n: r.chat.messages })}
        <small>{when(r.chat.updated_at)}</small>
      </span>
    </a>
  );
}

/** The last chat as a large card with its last lines and the way on (A and B). */
export function LeadCard({ lead, className }: { lead: Recent; className: string }) {
  return (
    <a className={className} href={`#/chat/${lead.chat.id}`} style={hueStyle(lead.name)} data-log-label="Weiterschreiben">
      <Portrait who={who(lead)} className="pic" />
      <span className="lead-text">
        <span className="who">
          {rich("system.hub.lastWith", { name: <b>{lead.name}</b> })}
          {lead.chat.title && <> · {lead.chat.title}</>}
        </span>
        <blockquote>{quote(lead, t("system.hub.noMessageInChat"))}</blockquote>
        <span className="lead-foot">
          <span className="meta num">
            {t("system.hub.messages", { n: lead.chat.messages })} · {when(lead.chat.updated_at)}
          </span>
          <GoOn />
        </span>
      </span>
    </a>
  );
}

/** Carry on, a new chat with the same figure, or all its chats (C and D). */
export function LeadActions({ lead, onNewChat }: { lead: Recent; onNewChat: (id: number) => void }) {
  return (
    <div className="btn-row">
      <a className="btn primary joy" href={`#/chat/${lead.chat.id}`} data-log-label="Weiterschreiben">
        {t("system.hub.goOn")}
        <Icon name="next" small />
      </a>
      <button type="button" className="btn" onClick={() => onNewChat(lead.chat.character_id)} data-log-label="Neuer Chat mit der Figur">
        <Icon name="plus" small />
        {t("system.hub.newChat")}
      </button>
      <a className="btn ghost" href={`#/char/${lead.chat.character_id}/chats`} data-log-label="Alle Chats der Figur">
        {t("system.hub.allChats")}
      </a>
    </div>
  );
}

/** Figures, chats and messages as entries of a list of figures (A, B and D). */
export function CountFacts({ data }: { data: Data }) {
  return (
    <>
      <div>
        <dt>{t("system.hub.figures")}</dt>
        <dd className="num">{num(data.characters.length)}</dd>
      </div>
      <div>
        <dt>{t("system.hub.chatsLabel")}</dt>
        <dd className="num">{num(data.chats)}</dd>
      </div>
      <div>
        <dt>{t("system.hub.messagesLabel")}</dt>
        <dd className="num">{num(data.messages)}</dd>
      </div>
    </>
  );
}
