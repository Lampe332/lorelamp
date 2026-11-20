import { useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { createPortal } from "react-dom";
import { useFormFactor } from "./app/formfactor";
import { download, errorText, num, postJson, when, type CharInfo, type Library as Lib } from "./api";
import { confirmDelete } from "./general/confirm";
import { t, texts } from "./i18n";
import { droppedFiles, startImport } from "./import/job";
import { LayoutSwitch } from "./library/LayoutSwitch";
import {
  DEFAULT_LIBRARY_LAYOUT,
  isLibraryLayout,
  LIBRARY_LAYOUT_EVENT,
  rememberLibraryLayout,
  storedLibraryLayout,
  takeRequestedLayout,
  type LibraryLayoutId,
} from "./library/layout";
import { typing, useCardPicking } from "./library/picking";
import { TagFilter, type TagCount } from "./library/TagFilter";
import { fold, prepare, queryWords, score } from "./search";
import { useSpot } from "./spot";
import { FigurePane } from "./tablet/FigurePane";
import { hueStyle, Icon, Portrait } from "./ui";

type Props = {
  library: Lib | null;
  loadError: string;
  /** Why the model is not running, if the app knows. */
  engineNote: string;
  onRetry: () => void;
  /** Continue the latest chat (or start the first one); `fresh` always starts a new one. */
  onChat: (character: CharInfo, fresh: boolean) => void;
  /** A message at the foot of the window, optionally with "Rückgängig". */
  onToast: (toast: { text: string; undo?: () => void }) => void;
};

/** How the cards are ordered. */
export type LibrarySort = "recent" | "name" | "new";
const SORTS: { id: LibrarySort; label: string }[] = [
  texts({ id: "recent" }, { label: "cards.lib.sort.recent" }),
  texts({ id: "name" }, { label: "cards.lib.sort.name" }),
  texts({ id: "new" }, { label: "cards.lib.sort.new" }),
];

/** Order and filter of the library, remembered on this device. `tags`: folded names, all must match. */
type View = { sort: LibrarySort; tags: string[]; favorites: boolean };
const VIEW_KEY = "rp-library-view";
const DEFAULT_VIEW: View = { sort: "recent", tags: [], favorites: false };

function loadView(): View {
  try {
    const v = JSON.parse(localStorage.getItem(VIEW_KEY) ?? "null") as Partial<View> | null;
    if (!v) return DEFAULT_VIEW;
    return {
      sort: SORTS.some((s) => s.id === v.sort) ? (v.sort as LibrarySort) : DEFAULT_VIEW.sort,
      tags: Array.isArray(v.tags)
        ? v.tags.filter((t): t is string => typeof t === "string")
        : // the one tag remembered before several could be chosen
          typeof (v as { tag?: unknown }).tag === "string"
          ? [tagKey((v as { tag: string }).tag)]
          : [],
      favorites: v.favorites === true,
    };
  } catch {
    return DEFAULT_VIEW;
  }
}

function saveView(v: View) {
  try {
    localStorage.setItem(VIEW_KEY, JSON.stringify(v));
  } catch {
    // Private windows and blocked storage: the order lasts as long as the page.
  }
}

const byName = (a: CharInfo, b: CharInfo) => a.name.localeCompare(b.name, "de", { sensitivity: "base" }) || a.id - b.id;
const ORDER: Record<LibrarySort, (a: CharInfo, b: CharInfo) => number> = {
  // Characters without a chat come after the ones chatted with, by name.
  recent: (a, b) => (b.last_chat_at ?? -1) - (a.last_chat_at ?? -1) || byName(a, b),
  name: byName,
  new: (a, b) => b.created_at - a.created_at || b.id - a.id,
};

/** One tag in any spelling: case, accents and umlauts do not matter. */
const tagKey = (t: string) => fold(t.trim()).text;

/** The line under the name: the own short text, else the automatic one, else the card's tags. */
const shortText = (c: CharInfo) => c.tagline || c.blurb || (c.tags.length ? c.tags.join(" · ") : t("cards.lib.noBlurb"));

/** Every tag of a card (the card's and the own ones), each spelling once. */
function cardTags(c: CharInfo): { key: string; name: string }[] {
  const out = new Map<string, string>();
  for (const t of [...c.own_tags, ...c.tags]) {
    const key = tagKey(t);
    if (key && !out.has(key)) out.set(key, t.trim());
  }
  return [...out].map(([key, name]) => ({ key, name }));
}

/** Export formats of cards (`POST /characters/export`). */
const EXPORTS = [
  { id: "png", label: "PNG" },
  { id: "json_v2", label: "JSON V2" },
  { id: "json_v3", label: "JSON V3" },
] as const;

export function Library({ library, loadError, engineNote, onRetry, onChat, onToast }: Props) {
  const [over, setOver] = useState(false);
  const [query, setQuery] = useState("");
  const [menu, setMenu] = useState<number | null>(null); // the card whose actions are showing
  // A tablet shows the tapped card's figure in a pane beside the cards (tablet/FigurePane) instead
  // of the actions on the card; the pane keeps the figure tapped last.
  const tablet = useFormFactor() === "tablet";
  const [paneId, setPaneId] = useState<number | null>(null);
  useEffect(() => {
    if (menu !== null) setPaneId(menu);
  }, [menu]);
  const [view, setView] = useState(loadView);
  const [picked, setPicked] = useState<Set<number> | null>(null); // selection mode: the picked cards
  const [step, setStep] = useState<"tag" | "export" | null>(null); // the selection bar's second step
  const [tagText, setTagText] = useState("");
  const [working, setWorking] = useState(false);
  // Stars set here, shown at once until the library has them.
  const [stars, setStars] = useState(new Map<number, boolean>());

  const change = (next: Partial<View>) =>
    setView((v) => {
      const out = { ...v, ...next };
      saveView(out);
      return out;
    });

  useEffect(() => setStars(new Map()), [library]);

  // The layout, kept like the start layout: this device's last one until the library is there, a
  // fresh choice until the server has it.
  const [storedLayout] = useState(storedLibraryLayout);
  const [pendingLayout, setPendingLayout] = useState<LibraryLayoutId | null>(null);
  const layout: LibraryLayoutId =
    pendingLayout ?? (library ? (isLibraryLayout(library.library_layout) ? library.library_layout : DEFAULT_LIBRARY_LAYOUT) : (storedLayout ?? DEFAULT_LIBRARY_LAYOUT));
  useEffect(() => {
    if (library) rememberLibraryLayout(layout);
  }, [library, layout]);
  const chooseLayout = async (id: LibraryLayoutId) => {
    setPendingLayout(id);
    rememberLibraryLayout(id);
    try {
      await postJson("/settings", { library_layout: id }, "PUT");
      onRetry(); // the library reloads with the saved layout
    } catch (e) {
      setPendingLayout(null);
      onToast({ text: t("cards.lib.layoutFailed", { error: errorText(e) }) });
    }
  };
  // The pending choice gives way once the library has it.
  useEffect(() => {
    if (library && pendingLayout === library.library_layout) setPendingLayout(null);
  }, [library, pendingLayout]);
  // A layout chosen in the quick actions (Ctrl+K), on arrival and while the tab is open.
  useEffect(() => {
    const take = () => {
      const id = takeRequestedLayout();
      if (id) void chooseLayout(id);
    };
    take();
    window.addEventListener(LIBRARY_LAYOUT_EVENT, take);
    return () => window.removeEventListener(LIBRARY_LAYOUT_EVENT, take);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // The card menu closes on Escape and on a click anywhere else.
  useEffect(() => {
    if (menu === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // Keyboard users land back on the card they opened, not at the top of the page.
      document.querySelector<HTMLButtonElement>(".cc.open .cc-open")?.focus();
      setMenu(null);
    };
    const onDown = (e: PointerEvent) => {
      if (!(e.target as Element).closest?.(".cc.open, .tb-pane")) setMenu(null);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("pointerdown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("pointerdown", onDown);
    };
  }, [menu]);

  // Enter and leave fire for every element passed on the way, so count them instead of toggling.
  const depth = useRef(0);
  const hasFiles = (e: DragEvent) => e.dataTransfer.types.includes("Files");
  const drop = (e: DragEvent) => {
    e.preventDefault();
    depth.current = 0;
    setOver(false);
    // Dropped files and folders are imported on the import page, which shows the progress (or the
    // import that still runs; a second one does not start beside it).
    void droppedFiles(e.dataTransfer).then((picked) => {
      if (!picked.length) return;
      startImport(picked);
      location.hash = "#/import";
    });
  };
  const dragOver = (e: DragEvent) => {
    if (hasFiles(e)) e.preventDefault();
  };
  const dragEnter = (e: DragEvent) => {
    if (!hasFiles(e)) return;
    depth.current += 1;
    setOver(true);
  };
  const dragLeave = () => {
    depth.current = Math.max(0, depth.current - 1);
    if (depth.current === 0) setOver(false);
  };

  const all = useMemo(
    () => (library?.characters ?? []).map((c) => (stars.has(c.id) ? { ...c, favorite: stars.get(c.id) ?? c.favorite } : c)),
    [library, stars],
  );
  // Each card's tags and what the search looks at: the name, the tags, and (weaker) the short text.
  const indexed = useMemo(
    () =>
      all.map((c) => {
        const tags = cardTags(c);
        return { c, tags, keys: new Set(tags.map((t) => t.key)), find: prepare(c.name, tags.map((t) => t.name).join(" "), shortText(c)) };
      }),
    [all],
  );
  // Every tag with how many cards carry it, the most used first.
  const tags = useMemo(() => {
    const counts = new Map<string, TagCount>();
    for (const { tags } of indexed) {
      for (const t of tags) {
        const e = counts.get(t.key);
        if (e) e.count += 1;
        else counts.set(t.key, { ...t, count: 1 });
      }
    }
    return [...counts.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name, "de", { sensitivity: "base" }));
  }, [indexed]);
  // A remembered tag that no card carries any more filters nothing.
  const chosen = useMemo(() => view.tags.filter((k) => tags.some((t) => t.key === k)), [view.tags, tags]);
  const q = query.trim();
  const shown = useMemo(() => {
    const words = queryWords(q);
    const hits: { c: CharInfo; s: number }[] = [];
    for (const { c, keys, find } of indexed) {
      if (view.favorites && !c.favorite) continue;
      if (!chosen.every((k) => keys.has(k))) continue;
      const m = q ? score(find, words, q) : { score: 0 };
      if (m) hits.push({ c, s: m.score });
    }
    // While searching, the best matches come first (in the chosen order among equals).
    return hits.sort((a, b) => b.s - a.s || ORDER[view.sort](a.c, b.c)).map((h) => h.c);
  }, [indexed, view.favorites, view.sort, chosen, q]);
  const filtered = view.favorites || chosen.length > 0 || q !== "";
  const tagsById = useMemo(() => new Map(indexed.map((x) => [x.c.id, x.tags])), [indexed]);

  // Cards deleted elsewhere leave the selection.
  useEffect(() => setPicked((p) => (p ? new Set(all.filter((c) => p.has(c.id)).map((c) => c.id)) : p)), [library]); // eslint-disable-line react-hooks/exhaustive-deps

  // Ctrl+K "Charaktere sortieren" points at the tools once they are on screen.
  useSpot(`library:${all.length > 0}`);

  const selecting = picked !== null;
  // Tablet: the figure in the pane — the card tapped last while it is among the shown ones, else the first.
  const paneFigure = tablet && !selecting ? (shown.find((c) => c.id === paneId) ?? shown[0] ?? null) : null;
  const pickedIds = all.filter((c) => picked?.has(c.id)).map((c) => c.id);
  const pickedCards = all.filter((c) => picked?.has(c.id));
  const allShownPicked = shown.length > 0 && shown.every((c) => picked?.has(c.id));
  // The card picked or unpicked last: Shift+click picks every shown card from there to the clicked one.
  const anchor = useRef<number | null>(null);
  const pick = (id: number, range: boolean) => {
    const ids = shown.map((c) => c.id);
    const from = anchor.current === null ? -1 : ids.indexOf(anchor.current);
    const to = ids.indexOf(id);
    anchor.current = id;
    setPicked((p) => {
      const next = new Set(p);
      if (range && from >= 0 && to >= 0) {
        for (const i of ids.slice(Math.min(from, to), Math.max(from, to) + 1)) next.add(i);
      } else if (!next.delete(id)) next.add(id);
      return next;
    });
  };
  const stopSelecting = () => {
    setPicked(null);
    setStep(null);
    setTagText("");
    anchor.current = null;
  };

  const viewRef = useRef<HTMLDivElement>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const frame = useCardPicking({
    view: viewRef,
    grid: gridRef,
    selecting,
    picked,
    setPicked,
    // A long press with a finger starts the selection with that card (or adds it).
    onHold: (id) => {
      anchor.current = id;
      setMenu(null);
      setPicked((p) => new Set(p).add(id));
    },
  });

  // Ctrl+A picks every card on screen (the filtered ones only), unless a text field has the focus.
  useEffect(() => {
    if (shown.length === 0) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== "a" || !(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey || e.defaultPrevented) return;
      if (typing(document.activeElement)) return;
      e.preventDefault();
      setMenu(null);
      setPicked(new Set(shown.map((c) => c.id)));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shown]);

  const setStar = async (c: CharInfo, on: boolean) => {
    setStars((m) => new Map(m).set(c.id, on));
    try {
      await postJson(`/characters/${c.id}/favorite`, { favorite: on }, "PUT");
    } catch (e) {
      setStars((m) => {
        const next = new Map(m);
        next.delete(c.id);
        return next;
      });
      onToast({ text: t("cards.lib.favFailed", { error: errorText(e) }) });
    }
  };

  /** One action on the picked cards. */
  const bulk = async (action: "delete" | "restore" | "tag" | "untag" | "favorite" | "unfavorite", tagName?: string, ids = pickedIds) => {
    setWorking(true);
    try {
      return await postJson<{ changed: number }>("/characters/bulk", { ids, action, tag: tagName });
    } catch (e) {
      onToast({ text: t("cards.lib.failed", { error: errorText(e) }) });
      return null;
    } finally {
      setWorking(false);
    }
  };

  const remove = async () => {
    const ids = pickedIds;
    if (!(await confirmDelete(t("cards.lib.cards", { n: ids.length })))) return;
    const r = await bulk("delete");
    if (!r) return;
    stopSelecting();
    onToast({
      text: t("cards.lib.deleted", { n: ids.length }),
      undo: () => {
        onToast({ text: t("cards.lib.restored", { n: ids.length }) });
        void bulk("restore", undefined, ids);
      },
    });
  };

  const applyTag = async (action: "tag" | "untag") => {
    const name = tagText.trim();
    if (!name) return;
    const r = await bulk(action, name);
    if (!r) return;
    setStep(null);
    setTagText("");
    onToast({ text: t(action === "tag" ? "cards.lib.tagAdded" : "cards.lib.tagRemoved", { name, n: r.changed }) });
  };

  const allFavorite = pickedCards.length > 0 && pickedCards.every((c) => c.favorite);
  const exportCards = async (format: string) => {
    setWorking(true);
    try {
      await download("/characters/export", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ ids: pickedIds, format }) }, t("cards.lib.zipName"));
      setStep(null);
    } catch (e) {
      onToast({ text: t("cards.exportFailed", { error: errorText(e) }) });
    } finally {
      setWorking(false);
    }
  };

  return (
    <div ref={viewRef} className="view" onDragOver={dragOver} onDragEnter={dragEnter} onDragLeave={dragLeave} onDrop={drop}>
      {over && (
        <div className="drop-overlay" aria-hidden="true">
          <div>
            <Icon name="upload" />
            <b>{t("cards.lib.drop")}</b>
            <span>{t("cards.lib.drop.text")}</span>
          </div>
        </div>
      )}
      <div className="page">
        <div className="title-row">
          <h1>{t("cards.lib.title")}</h1>
          {library && all.length > 0 && <span className="sub num">{t("cards.lib.cards", { n: all.length })}</span>}
          <div className="actions">
            {library && all.length > 0 && !selecting && (
              <button className="btn icon-sm" onClick={() => setPicked(new Set())} aria-label={t("cards.pick")} title={t("cards.lib.pick.title")}>
                <Icon name="select" small />
                <span className="hide-sm">{t("cards.pick")}</span>
              </button>
            )}
            {selecting ? (
              <button className="btn ghost" onClick={() => setPicked(allShownPicked ? new Set() : new Set(shown.map((c) => c.id)))}>
                {allShownPicked ? t("cards.pickNone") : t("cards.pickAll")}
              </button>
            ) : (
              <>
                <a className="btn" href="#/chars/new" aria-label={t("builder.entry")} data-spot="builder-new">
                  <Icon name="pencil" small />
                  <span className="hide-sm">{t("builder.entry")}</span>
                  <span className="show-sm">{t("builder.entry.short")}</span>
                </a>
                <a className="btn primary" href="#/import" aria-label={t("cards.lib.import")}>
                  <Icon name="plus" small />
                  <span className="hide-sm">{t("cards.lib.import")}</span>
                  <span className="show-sm">{t("cards.lib.import.short")}</span>
                </a>
              </>
            )}
          </div>
        </div>

        {engineNote && (
          <div className="well result" role="alert">
            <p>
              <b>{t("cards.lib.engine")}</b> {engineNote}
            </p>
          </div>
        )}

        {library && all.length > 0 && !selecting && (
          <p className="drop-hint">
            <Icon name="upload" small />
            {t("cards.lib.dropHint")}
          </p>
        )}

        {loadError && (
          <div className="well result" role="alert">
            <p>{t("cards.lib.loadFailed", { error: loadError })}</p>
            <button className="btn sm" onClick={onRetry}>
              {t("cards.lib.retry")}
            </button>
          </div>
        )}

        {!library && !loadError && (
          <div className={`grid-chars lib-${layout}`} aria-busy="true" aria-label={t("cards.lib.loading")}>
            {[0, 1, 2, 3].map((i) => (
              <div key={i} className="cc placeholder">
                <span className="art" />
                <div className="body">
                  <span className="bar-ph" />
                  <span className="bar-ph short" />
                </div>
              </div>
            ))}
          </div>
        )}

        {library && all.length === 0 && (
          <div className="empty drop-empty">
            <Icon name="upload" />
            <p>{t("cards.lib.empty")}</p>
            <p className="muted">{t("cards.lib.empty.note")}</p>
            <div className="btn-row">
              <a className="btn primary" href="#/import">
                {t("cards.lib.empty.import")}
              </a>
              <a className="btn" href="#/chars/new">
                <Icon name="pencil" small />
                {t("builder.entry")}
              </a>
            </div>
          </div>
        )}

        {library && all.length > 0 && (
          <div className="lib-tools" data-spot="library-order">
            <div className="toolbar">
              <label className="search lib-search">
                <Icon name="search" small />
                <input
                  className="input"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape" && query) {
                      e.stopPropagation();
                      setQuery("");
                    }
                  }}
                  placeholder={t("cards.lib.search")}
                  aria-label={t("cards.lib.search.label")}
                  data-log-label="Charaktere durchsuchen"
                />
              </label>
              <label className="lib-sort">
                <span className="sr">{t("cards.lib.sort")}</span>
                <Icon name="updown" small />
                <select className="input" value={view.sort} onChange={(e) => change({ sort: e.target.value as LibrarySort })} aria-label={t("cards.lib.sort")}>
                  {SORTS.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.label}
                    </option>
                  ))}
                </select>
              </label>
              <button
                className={`btn icon-sm lib-fav${view.favorites ? " on" : ""}`}
                aria-pressed={view.favorites}
                onClick={() => change({ favorites: !view.favorites })}
                title={t("cards.lib.favs.title")}
                aria-label={t("cards.lib.favs")}
              >
                <Icon name="star" small />
                <span className="lib-fav-text">{t("cards.lib.favs")}</span>
              </button>
              {filtered && (
                <span className="right num lib-count" role="status">
                  {t("cards.lib.shownOf", { shown: shown.length, all: all.length })}
                </span>
              )}
              <LayoutSwitch value={layout} onChange={(id) => void chooseLayout(id)} />
            </div>
            {tags.length > 0 && <TagFilter tags={tags} chosen={chosen} onChange={(next) => change({ tags: next })} />}
          </div>
        )}

        {library && all.length > 0 && shown.length === 0 && (
          <div className="empty-line lib-none">
            <p>
              {q ? t("cards.lib.none.query", { q }) : view.favorites && chosen.length === 0 ? t("cards.lib.none.favs") : t("cards.lib.none.filter")}
            </p>
            <button
              className="btn sm"
              onClick={() => {
                setQuery("");
                change({ tags: [], favorites: false });
              }}
            >
              {t("cards.lib.showAll")}
            </button>
          </div>
        )}

        {shown.length > 0 && (
          <div
            ref={gridRef}
            className={`grid-chars x3-cards lib-${layout}${selecting ? " picking" : ""}`}
            role={selecting ? "group" : undefined}
            aria-label={selecting ? t("cards.lib.pickCards") : undefined}
            onDragStart={selecting ? (e) => e.preventDefault() : undefined}
          >
            {shown.map((c) => {
              const open = menu === c.id;
              const on = !!picked?.has(c.id);
              const ownTags = tagsById.get(c.id) ?? [];
              const body =
                layout === "list" ? (
                  <>
                    <Portrait who={c} className="art" />
                    <span className="body">
                      <span className="lib-name">
                        <b>{c.name}</b>
                      </span>
                      <span className="s">{shortText(c)}</span>
                    </span>
                    <span className="lib-facts">
                      <span className="lib-tags">
                        {ownTags.slice(0, 3).map((t) => (
                          <span key={t.key} className="lib-tag">
                            {t.name}
                          </span>
                        ))}
                        {ownTags.length > 3 && <span className="lib-tag-more num">+{num(ownTags.length - 3)}</span>}
                      </span>
                      <span className="lib-when num">{c.last_chat_at ? when(c.last_chat_at) : t("cards.noChat")}</span>
                    </span>
                  </>
                ) : (
                  <>
                    <Portrait who={c} className="art" />
                    <span className="body">
                      <b>{c.name}</b>
                      <span className="s">{shortText(c)}</span>
                      <span className="meta num">{c.chats === 0 ? t("cards.noChat") : t("cards.chats", { n: c.chats })}</span>
                    </span>
                  </>
                );
              return (
                <article key={c.id} className={`cc ${open ? "open" : ""}${on ? " picked" : ""}${c.favorite ? " fav" : ""}${paneFigure?.id === c.id ? " tb-shown" : ""}`} style={hueStyle(c.name)} data-id={c.id}>
                  {selecting ? (
                    <label className="cc-open" data-log-label="Karte auswählen">
                      {body}
                      <span className="check cc-check">
                        <input type="checkbox" checked={on} onChange={(e) => pick(c.id, (e.nativeEvent as MouseEvent).shiftKey)} aria-label={t("cards.pickOne", { name: c.name })} data-log-label="Karte auswählen" />
                        <Icon name="check" small />
                      </span>
                    </label>
                  ) : (
                    <button className="cc-open" onClick={() => setMenu(open ? null : c.id)} aria-expanded={open} aria-label={t("cards.lib.showActions", { name: c.name })}>
                      {body}
                    </button>
                  )}
                  {!selecting && (
                    <button
                      className={`cc-star${c.favorite ? " on" : ""}`}
                      aria-pressed={c.favorite}
                      aria-label={t("cards.lib.star", { name: c.name })}
                      title={c.favorite ? t("cards.lib.star.on") : t("cards.lib.star.off")}
                      onClick={() => void setStar(c, !c.favorite)}
                    >
                      <Icon name="star" small />
                    </button>
                  )}
                  {open && !selecting && !tablet && (
                    <div
                      className="cc-menu"
                      role="group"
                      aria-label={t("cards.actionsFor", { name: c.name })}
                      onClick={(e) => e.target === e.currentTarget && setMenu(null)}
                    >
                      {c.chats > 0 ? (
                        <>
                          <button className="btn primary" autoFocus onClick={() => onChat(c, false)}>
                            {t("cards.lib.continue")}
                          </button>
                          <button className="btn solid" onClick={() => onChat(c, true)}>
                            {t("cards.newChat")}
                          </button>
                        </>
                      ) : (
                        <button className="btn primary" autoFocus onClick={() => onChat(c, false)}>
                          {t("cards.startChat")}
                        </button>
                      )}
                      <a className="btn solid" href={`#/char/${c.id}`}>
                        {t("cards.lib.settings")}
                      </a>
                    </div>
                  )}
                </article>
              );
            })}
          </div>
        )}

        {selecting && (
          <div className="savebar selbar lib-selbar" role="toolbar" aria-label={t("cards.selection")}>
            {step === "tag" ? (
              <form
                className="sel-step"
                onSubmit={(e) => {
                  e.preventDefault();
                  void applyTag("tag");
                }}
              >
                <input
                  className="input"
                  autoFocus
                  value={tagText}
                  maxLength={40}
                  list="lib-tag-list"
                  placeholder={t("cards.tag.placeholder")}
                  aria-label={t("cards.lib.tag")}
                  onChange={(e) => setTagText(e.target.value)}
                  onKeyDown={(e) => e.key === "Escape" && setStep(null)}
                />
                <datalist id="lib-tag-list">
                  {tags.map((t) => (
                    <option key={t.name} value={t.name} />
                  ))}
                </datalist>
                <button type="submit" className="btn primary" disabled={working || !tagText.trim() || pickedIds.length === 0}>
                  {t("cards.lib.tag.add")}
                </button>
                <button type="button" className="btn" disabled={working || !tagText.trim() || pickedIds.length === 0} onClick={() => void applyTag("untag")}>
                  {t("cards.lib.tag.remove")}
                </button>
                <button type="button" className="btn ghost" onClick={() => setStep(null)} aria-label={t("cards.back")} title={t("cards.back")}>
                  <Icon name="close" small />
                </button>
              </form>
            ) : step === "export" ? (
              <div className="sel-step" role="group" aria-label={t("cards.lib.exportAs")}>
                <span className="muted">{t("cards.lib.asZip")}</span>
                {EXPORTS.map((f) => (
                  <button key={f.id} className="btn" disabled={working || pickedIds.length === 0} onClick={() => void exportCards(f.id)}>
                    {f.label}
                  </button>
                ))}
                <button type="button" className="btn ghost" onClick={() => setStep(null)} aria-label={t("cards.back")} title={t("cards.back")}>
                  <Icon name="close" small />
                </button>
              </div>
            ) : (
              <>
                <span className="muted" role="status">
                  {pickedIds.length === 0 ? t("cards.nothingPicked") : t("cards.lib.picked", { n: pickedIds.length })}
                </span>
                <button className="btn ghost" onClick={stopSelecting} aria-label={t("cards.cancelPick")} title={t("cards.cancelPick")}>
                  <span className="hide-sm">{t("cards.cancel")}</span>
                  <span className="show-sm">
                    <Icon name="close" small />
                  </span>
                </button>
                <button
                  className="btn icon-sm"
                  disabled={working || pickedIds.length === 0}
                  onClick={() => void bulk(allFavorite ? "unfavorite" : "favorite")}
                  aria-label={allFavorite ? t("cards.lib.fav.remove") : t("cards.lib.fav.add")}
                  title={allFavorite ? t("cards.lib.fav.remove") : t("cards.lib.star.off")}
                >
                  <Icon name="star" small />
                  <span className="hide-sm">{allFavorite ? t("cards.lib.fav.no") : t("cards.lib.fav.yes")}</span>
                </button>
                <button className="btn icon-sm" disabled={pickedIds.length === 0} onClick={() => setStep("tag")} aria-label={t("cards.lib.tagStep")} title={t("cards.lib.tagStep")}>
                  <Icon name="tag" small />
                  <span className="hide-sm">{t("cards.lib.tagStep.short")}</span>
                </button>
                <button className="btn icon-sm" disabled={pickedIds.length === 0} onClick={() => setStep("export")} aria-label={t("cards.export")} title={t("cards.lib.export.title")}>
                  <Icon name="download" small />
                  <span className="hide-sm">{t("cards.export")}</span>
                </button>
                <button className="btn icon-sm danger" disabled={working || pickedIds.length === 0} onClick={() => void remove()} aria-label={t("cards.lib.delete")} title={t("cards.lib.delete.title")}>
                  <Icon name="trash" small />
                  <span className="hide-sm">{t("cards.lib.delete")}</span>
                </button>
              </>
            )}
          </div>
        )}
      </div>
      {paneFigure && <FigurePane character={paneFigure} open={menu !== null} rev={library} onChat={onChat} onClose={() => setMenu(null)} />}
      {selecting && createPortal(<div ref={frame} className="x3-frame" hidden aria-hidden="true" />, document.body)}
    </div>
  );
}
