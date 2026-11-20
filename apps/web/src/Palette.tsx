// Quick actions (Ctrl+K): one field that finds characters, chats, pages and settings and runs them
// (the commands themselves come from commands.ts).
// Only real actions: everything here does what it says at once. The same list serves the window
// (Palette) and the command line of the "Kommandozentrale" start layout (CommandField).
// Search is local and forgiving (search.ts): settings are found by rough descriptions ("dunkel",
// "mein name", "was ist passiert") and open at their place in the settings (spot.ts).

import { t } from "./i18n";
import { rich } from "./i18n/rich";
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { api, MARK_END, MARK_START, when, type SearchHit } from "./api";
import { formFactor } from "./app/formfactor";
import type { Command } from "./commands";
import { jumpToMessage } from "./jump";
import { logEvent } from "./log";
import { fold, prepare, queryWords, score, type Marks, type Prepared } from "./search";
import { Icon, Portrait } from "./ui";

type Hit = { c: Command; marks: Marks };
type Section = { group: string; hits: Hit[] };

const MAX_HITS = 40;

/** What to show for a query, in sections. Before typing: the short list. While typing: the best
 *  matches; sections follow their best hit. After a step ("Farbschema "): only that step's choices. */
function search(all: Command[], prepared: Map<string, Prepared>, query: string): Section[] {
  let pool = all;
  let rest = query;
  const q = fold(query).text;
  const step = all.find((c) => c.fills && q.startsWith(fold(c.fills).text));
  if (step?.fills) {
    const head = fold(step.fills).text;
    pool = all.filter((c) => c !== step && fold(c.label).text.startsWith(head));
    rest = query.slice(step.fills.length);
  }
  const words = queryWords(rest);
  let hits: (Hit & { s: number; i: number })[];
  if (words.length === 0) {
    hits = step
      ? pool.map((c, i) => ({ c, marks: [], s: 0, i }))
      : pool.filter((c) => c.idle !== undefined).map((c, i) => ({ c, marks: [], s: -(c.idle ?? 0), i }));
  } else {
    hits = [];
    pool.forEach((c, i) => {
      if (c.fills && !step && q.startsWith(fold(c.fills).text)) return;
      const r = score(prepared.get(c.id) ?? prepare(c.label), words, rest);
      if (r) hits.push({ c, marks: r.marks, s: r.score + (c.boost ?? 0), i });
    });
  }
  hits.sort((a, b) => b.s - a.s || a.i - b.i);
  const sections: Section[] = [];
  for (const h of hits.slice(0, MAX_HITS)) {
    const sec = sections.find((s) => s.group === h.c.group);
    if (sec) sec.hits.push(h);
    else sections.push({ group: h.c.group, hits: [h] });
  }
  return sections;
}

/** Messages of all chats are searched on the server from this many letters on, after a pause in typing. */
const CHAT_SEARCH_FROM = 2;
const CHAT_SEARCH_WAIT = 220;
const CHAT_HITS = 8;

/** A message found in a chat as an entry of the list: its snippet with the matched words marked. */
function hitCommand(h: SearchHit): Hit {
  let label = "";
  const marks: Marks = [];
  let open = -1;
  for (const ch of h.snippet) {
    if (ch === MARK_START) open = label.length;
    else if (ch === MARK_END) {
      if (open >= 0 && label.length > open) marks.push([open, label.length]);
      open = -1;
    } else label += /\s/.test(ch) ? " " : ch;
  }
  return {
    marks,
    c: {
      id: `msg-${h.message_id}`,
      group: "In Chats",
      label,
      where: `${h.character}, ${when(h.created_at)}`,
      who: { id: h.character_id, name: h.character, has_avatar: h.has_avatar, avatar_rev: h.avatar_rev, avatar_pos: h.avatar_pos },
      kind: "chat-treffer",
      run: () => jumpToMessage(h.chat_id, h.message_id),
    },
  };
}

/** Messages that match the query, from the server (empty while it is short or a step is chosen). */
function useChatHits(query: string, off: boolean): Hit[] {
  const [hits, setHits] = useState<{ q: string; hits: Hit[] }>({ q: "", hits: [] });
  const q = query.trim();
  const wanted = !off && [...q].filter((ch) => /[\p{L}\p{N}]/u.test(ch)).length >= CHAT_SEARCH_FROM;
  useEffect(() => {
    if (!wanted) return;
    let live = true;
    const t = window.setTimeout(() => {
      api<{ hits: SearchHit[] }>(`/search/messages?q=${encodeURIComponent(q)}&limit=${CHAT_HITS}`)
        .then((r) => live && setHits({ q, hits: r.hits.map(hitCommand) }))
        .catch(() => live && setHits({ q, hits: [] }));
    }, CHAT_SEARCH_WAIT);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [q, wanted]);
  // Hits of an older query stay until the new ones are there, so the list does not jump while typing.
  return wanted ? hits.hits : [];
}

/** The label with the matched letters set apart. */
function Marked({ text, marks }: { text: string; marks: Marks }) {
  if (marks.length === 0) return <>{text}</>;
  const parts: ReactNode[] = [];
  let at = 0;
  marks.forEach(([a, b], i) => {
    if (a > at) parts.push(text.slice(at, a));
    parts.push(<mark key={i}>{text.slice(a, b)}</mark>);
    at = b;
  });
  if (at < text.length) parts.push(text.slice(at));
  return <>{parts}</>;
}

/** Examples that show the search understands rough descriptions; a click puts them in the field. */
const examples = () => [t("shell.pal.example.1"), t("shell.pal.example.2"), t("shell.pal.example.3"), t("shell.pal.example.4")];

type ListProps = {
  commands: Command[];
  query: string;
  setQuery: (q: string) => void;
  onDone: () => void;
  /** The text field's label and placeholder. */
  placeholder: string;
  autoFocus?: boolean;
  /** The command line on the start screen: the results open under it while it has the focus. */
  inline?: boolean;
};

/** The field and its list of results, driven by the keyboard (arrows, Enter, Escape). */
function CommandList({ commands, query, setQuery, onDone, placeholder, autoFocus, inline }: ListProps) {
  const prepared = useMemo(() => new Map(commands.map((c) => [c.id, prepare(c.label, c.keywords, c.loose)])), [commands]);
  const stepChosen = commands.some((c) => c.fills && fold(query).text.startsWith(fold(c.fills).text));
  const chatHits = useChatHits(query, stepChosen);
  const sections = useMemo(() => {
    const all = search(commands, prepared, query);
    if (chatHits.length > 0) all.push({ group: "In Chats", hits: chatHits });
    // The start screen lists the recent chats right below its command line; its short list skips them.
    return inline && query.trim() === "" ? all.filter((s) => s.group !== "Chats") : all;
  }, [commands, prepared, query, inline, chatHits]);
  const flat = useMemo(() => sections.flatMap((s) => s.hits), [sections]);
  const [active, setActive] = useState(0);
  const [focused, setFocused] = useState(false);
  const listId = useId();
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const typed = query.trim() !== "";
  const open = !inline || focused;

  const run = (c: Command) => {
    logEvent("info", "palette", `Schnellaktion: ${c.kind}`);
    if (c.fills) {
      setQuery(c.fills);
      input.current?.focus();
      return;
    }
    c.run?.();
    if (inline) input.current?.blur();
    onDone();
  };

  const onKey = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (flat.length === 0) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive((a) => (a + step + flat.length) % flat.length);
    } else if (e.key === "Home" && flat.length > 0 && e.ctrlKey) {
      e.preventDefault();
      setActive(0);
    } else if (e.key === "End" && flat.length > 0 && e.ctrlKey) {
      e.preventDefault();
      setActive(flat.length - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const h = flat[active];
      if (h) run(h.c);
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (query) setQuery("");
      else if (inline) input.current?.blur();
      else onDone();
    }
  };

  const count = flat.length;
  let index = -1;
  return (
    <>
      <div className="cmd-field">
        <Icon name="search" />
        <input
          ref={input}
          className="cmd-input"
          role="combobox"
          aria-expanded={open}
          aria-controls={listId}
          aria-activedescendant={open && flat[active] ? `${listId}-${active}` : undefined}
          aria-autocomplete="list"
          aria-label={placeholder}
          placeholder={placeholder}
          value={query}
          autoFocus={autoFocus}
          spellCheck={false}
          autoComplete="off"
          enterKeyHint="go"
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
        />
        {typed && (
          <button
            type="button"
            className="cmd-clear"
            aria-label={t("shell.pal.clear")}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              setQuery("");
              input.current?.focus();
            }}
          >
            <Icon name="close" small />
          </button>
        )}
        {inline ? !focused && <kbd aria-hidden="true">{t("shell.key.ctrlK")}</kbd> : <kbd aria-hidden="true">Esc</kbd>}
      </div>
      {open && (
        <div className="cmd-pop" onMouseDown={(e) => e.preventDefault() /* keep the focus in the field */}>
          <div className="cmd-list" id={listId} role="listbox" aria-label={t("shell.pal.results")} ref={list}>
            {sections.map((sec, si) => (
              // The id uses the position: a group name can hold a space ("In Chats"), which aria-labelledby would split.
              <div key={sec.group} className="cmd-part" role="group" aria-labelledby={`${listId}-g${si}`} data-group={sec.group}>
                <div className="cmd-group" id={`${listId}-g${si}`}>
                  {groupTitle(sec.group, !typed)}
                </div>
                {sec.hits.map(({ c, marks }) => {
                  const i = ++index;
                  return (
                    <div
                      key={c.id}
                      id={`${listId}-${i}`}
                      role="option"
                      aria-selected={i === active}
                      data-index={i}
                      className="cmd-item"
                      onMouseMove={() => i !== active && setActive(i)}
                      onClick={() => run(c)}
                    >
                      <span className="cmd-ico" aria-hidden="true">
                        {c.who ? (
                          <Portrait who={c.who} className="av" kind={c.whoKind} />
                        ) : c.swatch ? (
                          <span className="cmd-swatch" data-theme={c.swatch}>
                            <i />
                          </span>
                        ) : (
                          <Icon name={c.icon ?? "next"} />
                        )}
                      </span>
                      <span className="cmd-text">
                        <span className="cmd-label">
                          <Marked text={c.label} marks={typed ? marks : []} />
                        </span>
                        {c.hint && <span className="cmd-hint">{c.hint}</span>}
                      </span>
                      {c.current && (
                        <span className="cmd-now">
                          <Icon name="check" small />
                          {t("shell.pal.active")}
                        </span>
                      )}
                      {c.where && !c.current && <span className="cmd-where">{c.group === "Einstellungen" ? t("shell.pal.inSettings", { where: c.where }) : c.where}</span>}
                      <span className="cmd-go" aria-hidden="true">
                        {c.fills ? t("shell.pal.pick") : t("shell.pal.open")}
                        <kbd>Enter</kbd>
                      </span>
                    </div>
                  );
                })}
              </div>
            ))}
            {count === 0 && (
              <div className="cmd-none">
                <p>{rich("shell.pal.none", { query: <b>{t("shell.pal.quoted", { query: query.trim() })}</b> })}</p>
                <p className="help">{t("shell.pal.describe")}</p>
                <Examples onPick={(q) => { setQuery(q); input.current?.focus(); }} />
              </div>
            )}
            {!typed && count > 0 && (
              <div className="cmd-tip">
                <span>{t("shell.pal.tip")}</span>
                <Examples onPick={(q) => { setQuery(q); input.current?.focus(); }} />
              </div>
            )}
          </div>
          <div className="cmd-foot" aria-hidden="true">
            <span>
              <kbd>↑</kbd>
              <kbd>↓</kbd> {t("shell.pal.foot.select")}
            </span>
            <span>
              <kbd>Enter</kbd> {t("shell.pal.foot.open")}
            </span>
            <span>
              <kbd>Esc</kbd> {query ? t("shell.pal.foot.clear") : t("shell.pal.foot.close")}
            </span>
            <span className="cmd-count">{typed ? t("shell.pal.hits", { n: count }) : t("shell.pal.suggestions")}</span>
          </div>
          <span className="sr-only" role="status" aria-live="polite">
            {typed ? (count === 0 ? t("shell.pal.nothing") : t("shell.pal.hits", { n: count })) : ""}
          </span>
        </div>
      )}
    </>
  );
}

/** A section's title. A command's `group` is the section's id (German, also `data-group` and the
 *  log); this is its name in the app's language, and before typing the short list's own title. */
const GROUPS = ["Weiter", "Aktionen", "Einstellungen", "Chats", "Figuren", "Seiten", "Farbschema", "Persona", "Start-Layout", "In Chats"] as const;
const IDLE_GROUPS = ["Weiter", "Chats", "Aktionen", "Einstellungen"] as const;
const among = <T extends string>(list: readonly T[], v: string): v is T => (list as readonly string[]).includes(v);
function groupTitle(group: string, idle: boolean): string {
  if (idle && among(IDLE_GROUPS, group)) return t(`shell.pal.idle.${group}`);
  return among(GROUPS, group) ? t(`shell.pal.group.${group}`) : group;
}

function Examples({ onPick }: { onPick: (q: string) => void }) {
  return (
    <span className="cmd-examples">
      {examples().map((q) => (
        <button key={q} type="button" className="cmd-example" tabIndex={-1} onClick={() => onPick(q)}>
          {q}
        </button>
      ))}
    </span>
  );
}

/** The quick actions window. Focus goes into the field and comes back where it was. */
export function Palette({ commands, initial, onClose }: { commands: Command[]; initial: string; onClose: () => void }) {
  const [query, setQuery] = useState(initial);
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    return () => before?.focus?.({ preventScroll: true });
  }, []);

  // On a phone or tablet the window is a step in the history of its own, so the back key closes it
  // instead of leaving the page under it. A chosen action then runs once that step is gone, so a
  // page it opens takes the step's place. A window on the desk has no back key and stays as it was.
  const [stepped] = useState(() => formFactor() !== "desktop");
  const after = useRef<(() => void) | null>(null);
  const leaving = useRef(false);
  useEffect(() => {
    if (!stepped) return;
    if (!history.state?.cmdStep) history.pushState({ ...history.state, cmdStep: true }, "");
    const gone = () => {
      onClose();
      after.current?.();
    };
    window.addEventListener("popstate", gone);
    return () => window.removeEventListener("popstate", gone);
  }, [stepped, onClose]);
  const close = (then?: () => void) => {
    if (leaving.current) return;
    if (stepped && history.state?.cmdStep) {
      leaving.current = true;
      after.current = then ?? null;
      history.back();
    } else {
      onClose();
      then?.();
    }
  };
  const listed = useMemo(() => (stepped ? commands.map((c) => (c.run ? { ...c, run: () => close(c.run) } : c)) : commands), [stepped, commands]);

  return (
    <div className="cmd-scrim" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div
        className="cmd-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={t("shell.nav.searchLabel")}
        onKeyDown={(e) => {
          if (e.key === "Tab") e.preventDefault(); // the field keeps the focus; arrows move through the list
        }}
      >
        <CommandList commands={listed} query={query} setQuery={setQuery} onDone={() => close()} placeholder={t("shell.pal.placeholder")} autoFocus />
      </div>
    </div>
  );
}

/** The command line in the page (start layout "Kommandozentrale"): results open under it while it has the focus. */
export function CommandField({ commands }: { commands: Command[] }) {
  const [query, setQuery] = useState("");
  return (
    <div className="cmd-inline">
      <CommandList commands={commands} query={query} setQuery={setQuery} onDone={() => setQuery("")} placeholder={t("shell.pal.placeholder")} inline />
    </div>
  );
}
