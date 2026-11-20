// The screens of one character: shared header with tabs, the chat overview, the card editor.

import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  api,
  ApiError,
  chatName,
  download,
  errorText,
  postJson,
  snippet,
  tzOffset,
  when,
  type Card,
  type Character,
  type CharInfo,
  type ChatInfo,
  type ExportFormat,
  type Persona,
} from "./api";
import { t, texts } from "./i18n";
import { rich } from "./i18n/rich";
import { OwnTags } from "./OwnTags";
import { DefaultPersona } from "./persona/DefaultPersona";
import { useSpot, useSpotAsked } from "./spot";
import type { Picked } from "./image";
import { PictureField } from "./PictureField";
import { UserFit } from "./UserFit";
import { hueStyle, Icon, Portrait, type IconName } from "./ui";

export { CharTools, ChatCardExport } from "./CardExport";

export type CharTab = "chats" | "settings";

type HeaderProps = {
  character: CharInfo | null;
  /** The screen shown; null on a page that is none of the tabs (a chat's own card). */
  tab: CharTab | null;
  /** Line under the name. */
  sub: ReactNode;
  /** The card editor holds changes that are not saved yet. */
  unsaved?: boolean;
  /** Label for screen readers; the arrow leads one level up. */
  backLabel: string;
  onBack: () => void;
  children?: ReactNode;
};

/** Who the character is, on a field of its own hue, with the tabs of its screens along the bottom edge. */
export function CharHeader({ character, tab, sub, unsaved, backLabel, onBack, children }: HeaderProps) {
  const link = (to: CharTab) => (character ? `#/char/${character.id}/${to}` : undefined);
  return (
    <header className={`char-head${character ? "" : " loading"}`} style={character ? hueStyle(character.name) : undefined}>
      <div className="chat-head">
        <button className="icon-btn back" onClick={onBack} aria-label={backLabel} title={backLabel}>
          <Icon name="back" />
        </button>
        {character ? (
          <>
            <Portrait who={character} className="av" full />
            <div className="t">
              <h1>{character.name}</h1>
              <span className="num">{sub}</span>
            </div>
            {children && <div className="head-tools">{children}</div>}
          </>
        ) : (
          <>
            <span className="av ph" />
            <div className="t" aria-hidden="true">
              <span className="bar-ph short" />
            </div>
          </>
        )}
      </div>
      <nav className="tabs" aria-label={t("cards.head.areas")}>
        <a href={link("chats")} aria-current={tab === "chats" ? "page" : undefined}>
          <Icon name="chat" small />
          {t("cards.head.chats")}
        </a>
        <a href={link("settings")} aria-current={tab === "settings" ? "page" : undefined}>
          <Icon name="sliders" small />
          {t("cards.head.settings")}
          {unsaved && <i className="unsaved" title={t("cards.unsaved")} />}
        </a>
      </nav>
    </header>
  );
}

// ---------------------------------------------------------------- chat overview

type ChatsProps = {
  character: CharInfo;
  /** Changes whenever chats may have changed (another device, a new message). */
  rev: unknown;
  onNew: () => Promise<void>;
  onRename: (chat: ChatInfo, title: string) => void;
  onDelete: (chat: ChatInfo) => void;
};

/** All chats with one character. Nothing is written here; a row opens its chat. A row's menu exports
 *  it; "Auswählen" picks several for one ZIP. */
export function CharChats({ character, rev, onNew, onRename, onDelete }: ChatsProps) {
  const [renaming, setRenaming] = useState<{ id: number; text: string } | null>(null);
  const [chats, setChats] = useState<ChatInfo[] | null>(null);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);
  const [menu, setMenu] = useState<{ chat: ChatInfo; anchor: HTMLElement } | null>(null);
  // The chats the export dialog is open for; `zip`: several (or picked) chats in one file.
  const [exporting, setExporting] = useState<{ chats: ChatInfo[]; zip: boolean } | null>(null);
  // Selection mode: the picked chats (null = off).
  const [picked, setPicked] = useState<Set<number> | null>(null);
  const [asked, dismiss] = useSpotAsked("chat-export");
  useSpot(`chats:${character.id}:${asked}`);

  useEffect(() => {
    let stale = false;
    api<{ chats: ChatInfo[] }>(`/characters/${character.id}/chats`)
      .then((r) => {
        if (stale) return;
        setChats(r.chats);
        setError("");
        // Chats that are gone (deleted on another device) leave the selection.
        setPicked((p) => p && new Set(r.chats.filter((c) => p.has(c.id)).map((c) => c.id)));
      })
      .catch((e) => !stale && setError(errorText(e)));
    return () => {
      stale = true;
    };
  }, [character.id, rev]);

  // Another character's page starts without a selection.
  useEffect(() => setPicked(null), [character.id]);

  const create = async () => {
    setCreating(true);
    await onNew();
    setCreating(false);
  };

  const selecting = picked !== null;
  const toggle = (id: number) =>
    setPicked((p) => {
      const next = new Set(p);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  const allPicked = !!chats && !!picked && chats.length > 0 && picked.size === chats.length;
  const pickedChats = chats && picked ? chats.filter((c) => picked.has(c.id)) : [];

  return (
    <div className="view">
      <div className="page chats-page">
        <div className="chats-main">
          <div className="title-row">
            <h2 className="sec">{chats ? t("cards.chats", { n: chats.length }) : t("cards.head.chats")}</h2>
            {!selecting && chats && chats.length > 1 && <span className="sub hide-sm">{t("cards.chats.nameHint")}</span>}
            <div className="actions">
              {selecting ? (
                <button className="btn ghost" onClick={() => setPicked(allPicked ? new Set() : new Set(chats?.map((c) => c.id)))}>
                  {allPicked ? t("cards.pickNone") : t("cards.pickAll")}
                </button>
              ) : (
                <>
                  {chats && chats.length > 0 && (
                    <button className="btn icon-sm" onClick={() => setPicked(new Set())} aria-label={t("cards.pick")} title={t("cards.chats.pick.title")}>
                      <Icon name="select" small />
                      <span className="hide-sm">{t("cards.pick")}</span>
                    </button>
                  )}
                  <button className="btn primary" onClick={() => void create()} disabled={creating}>
                    <Icon name="plus" small />
                    {t("cards.newChat")}
                  </button>
                </>
              )}
            </div>
          </div>

          {asked && (
            <div className="well export-hint" data-spot="chat-export" role="note">
              <Icon name="download" small />
              <p>
                {chats && chats.length === 0
                  ? t("cards.chats.hint.none", { name: character.name })
                  : rich("cards.chats.hint.text", {
                      title: <b>{t("cards.chats.hint.title")}</b>,
                      dots: (
                        <span className="kbd-dots" aria-label={t("cards.chats.more")}>
                          ⋯
                        </span>
                      ),
                      pick: (
                        <span className="kbd-dots" aria-hidden="true">
                          <Icon name="select" small />
                        </span>
                      ),
                    })}
              </p>
              <button className="icon-btn" onClick={dismiss} aria-label={t("cards.chats.hint.close")} title={t("cards.close")}>
                <Icon name="close" small />
              </button>
            </div>
          )}

          {error && (
            <div className="well result" role="alert">
              <p>{t("cards.chats.loadFailed", { error })}</p>
            </div>
          )}

          {!chats && !error && (
            <div className="chat-list" aria-busy="true">
              {[0, 1].map((i) => (
                <div className="row-card chat-row" key={i}>
                  <span className="bar-ph" />
                </div>
              ))}
            </div>
          )}

          {chats && chats.length === 0 && (
            <div className="empty">
              <p>{t("cards.chats.empty", { name: character.name })}</p>
              <p className="muted">{t("cards.chats.empty.note")}</p>
              <button className="btn" onClick={() => void create()} disabled={creating}>
                <Icon name="plus" small />
                {t("cards.chats.first")}
              </button>
            </div>
          )}

          {chats && chats.length > 0 && (
            <div className={`chat-list${selecting ? " picking" : ""}`} role={selecting ? "group" : undefined} aria-label={selecting ? t("cards.chats.pickList") : undefined}>
              {chats.map((c) => {
                const commit = () => {
                  if (renaming?.id !== c.id) return;
                  const title = renaming.text.trim();
                  setRenaming(null);
                  if (title !== c.title) onRename(c, title);
                };
                const on = !!picked?.has(c.id);
                const lines = (
                  <>
                    <span className="l1">
                      <b className={c.title ? "named" : undefined}>{chatName(c)}</b>
                      {c.custom_card && <span className="tag">{t("cards.chats.ownCard")}</span>}
                    </span>
                    <span className="l2">{snippet(c.last) || t("cards.chats.noMessage")}</span>
                  </>
                );
                return (
                  <div className={`row-card chat-row${on ? " picked" : ""}`} key={c.id}>
                    {selecting ? (
                      <label className="chat-open pick" data-log-label="Chat auswählen">
                        <span className="check">
                          <input
                            type="checkbox"
                            checked={on}
                            onChange={() => toggle(c.id)}
                            aria-label={t("cards.pickOne", { name: chatName(c) })}
                            data-log-label="Chat auswählen"
                          />
                          <Icon name="check" small />
                        </span>
                        <span className="pick-text">{lines}</span>
                      </label>
                    ) : renaming?.id === c.id ? (
                      <form
                        className="rename"
                        onSubmit={(e) => {
                          e.preventDefault();
                          commit();
                        }}
                      >
                        <input
                          className="input"
                          autoFocus
                          value={renaming.text}
                          maxLength={80}
                          placeholder={t("cards.chats.rename.placeholder")}
                          aria-label={t("cards.chats.rename.label")}
                          onChange={(e) => setRenaming({ id: c.id, text: e.target.value })}
                          onBlur={commit}
                          onKeyDown={(e) => e.key === "Escape" && setRenaming(null)}
                        />
                        <span className="help">{t("cards.chats.rename.help")}</span>
                      </form>
                    ) : (
                      <a className="chat-open" href={`#/chat/${c.id}`}>
                        {lines}
                      </a>
                    )}
                    <span className="r num">
                      {t("cards.messages", { n: c.messages })}
                      <small>{t("cards.chats.last", { when: when(c.updated_at) })}</small>
                    </span>
                    {!selecting && (
                      <span className="row-actions">
                        <button
                          className="icon-btn"
                          onClick={() => setRenaming({ id: c.id, text: c.title })}
                          aria-label={t("cards.chats.rename.for", { name: chatName(c) })}
                          title={t("cards.chats.rename")}
                        >
                          <Icon name="pencil" small />
                        </button>
                        <a
                          className="icon-btn"
                          href={`#/chat/${c.id}/card`}
                          aria-label={t("cards.chats.card.for", { name: chatName(c) })}
                          title={t("cards.chats.card")}
                        >
                          <Icon name="sliders" small />
                        </a>
                        <button className="icon-btn" onClick={() => onDelete(c)} aria-label={t("cards.chats.delete.for", { name: chatName(c) })} title={t("cards.chats.delete")}>
                          <Icon name="trash" small />
                        </button>
                        <button
                          className="icon-btn"
                          aria-haspopup="menu"
                          aria-expanded={menu?.chat.id === c.id}
                          aria-label={t("cards.chats.more.for", { name: chatName(c) })}
                          title={t("cards.chats.more")}
                          onClick={(e) => setMenu(menu?.chat.id === c.id ? null : { chat: c, anchor: e.currentTarget })}
                        >
                          <Icon name="more" small />
                        </button>
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          {selecting && (
            <div className="savebar selbar" role="toolbar" aria-label={t("cards.selection")}>
              <span className="muted" role="status">
                {picked.size === 0
                  ? t("cards.nothingPicked")
                  : rich("cards.chats.picked", {
                      n: picked.size,
                      of: <span className="hide-sm"> {t("cards.chats.of", { n: chats?.length ?? 0 })}</span>,
                    })}
              </span>
              <button className="btn ghost" onClick={() => setPicked(null)} aria-label={t("cards.cancelPick")} title={t("cards.cancelPick")}>
                <span className="hide-sm">{t("cards.cancel")}</span>
                <span className="show-sm">
                  <Icon name="close" small />
                </span>
              </button>
              <button className="btn primary" disabled={picked.size === 0} onClick={() => setExporting({ chats: pickedChats, zip: true })}>
                <Icon name="download" small />
                {t("cards.exportZip")}
              </button>
            </div>
          )}
        </div>
        {/* What the card says about itself, beside the stories told with it (wide windows only). */}
        <aside className="about" aria-label={t("cards.chats.about.for", { name: character.name })}>
          <h2 className="sec">{t("cards.chats.about")}</h2>
          <p className={character.tagline || character.blurb ? "about-blurb" : "about-blurb faint"}>{character.tagline || character.blurb || t("cards.chats.about.none")}</p>
          {character.tags.length > 0 && (
            <ul className="about-tags" aria-label={t("cards.chats.tags")}>
              {character.tags.map((tag) => (
                <li className="tag" key={tag}>
                  {tag}
                </li>
              ))}
            </ul>
          )}
          <a className="btn sm" href={`#/char/${character.id}/settings`}>
            <Icon name="card" small />
            {t("cards.editCard")}
          </a>
        </aside>
      </div>

      {menu && (
        <RowMenu
          label={t("cards.actionsFor", { name: chatName(menu.chat) })}
          head={chatName(menu.chat)}
          anchor={menu.anchor}
          items={[
            { label: t("cards.exportDots"), sub: t("cards.chats.menu.export.sub"), icon: "download", onPick: () => setExporting({ chats: [menu.chat], zip: false }) },
            { label: t("cards.pick"), sub: t("cards.chats.menu.pick.sub"), icon: "select", onPick: () => setPicked(new Set([menu.chat.id])) },
          ]}
          onClose={(refocus) => {
            if (refocus) menu.anchor.focus();
            setMenu(null);
          }}
        />
      )}

      {exporting && (
        <ExportDialog
          character={character}
          chats={exporting.chats}
          zip={exporting.zip}
          onClose={() => setExporting(null)}
          onDone={() => {
            setExporting(null);
            if (exporting.zip) setPicked(null);
          }}
        />
      )}
    </div>
  );
}

type RowMenuItem = { label: string; sub: string; icon: IconName; onPick: () => void };

type RowMenuProps = {
  label: string;
  head: string;
  items: RowMenuItem[];
  anchor: HTMLElement;
  /** `refocus`: give the focus back to the button (Escape, a pick), not when a click went elsewhere. */
  onClose: (refocus: boolean) => void;
};

/** A row's menu: below its button on a desktop (above it when there is no room), a sheet on a phone. */
function RowMenu({ label, head, items, anchor, onClose }: RowMenuProps) {
  const [sheet] = useState(() => matchMedia("(max-width: 767px)").matches);
  const [place, setPlace] = useState<CSSProperties | undefined>(undefined);
  const box = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (sheet) return;
    const fit = () => {
      const r = anchor.getBoundingClientRect();
      const width = Math.min(296, window.innerWidth - 16);
      const left = Math.min(Math.max(r.right - width, 8), window.innerWidth - 8 - width);
      const height = box.current?.offsetHeight ?? 0;
      const below = window.innerHeight - r.bottom - 6 - 8;
      setPlace(below >= height || below >= r.top ? { left, width, top: r.bottom + 6 } : { left, width, bottom: window.innerHeight - r.top + 6 });
    };
    fit();
    window.addEventListener("resize", fit);
    // A scroll moves the button away from its menu: close it, as native menus do.
    const away = () => onClose(false);
    window.addEventListener("scroll", away, true);
    window.addEventListener("hashchange", away);
    return () => {
      window.removeEventListener("resize", fit);
      window.removeEventListener("scroll", away, true);
      window.removeEventListener("hashchange", away);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [anchor, sheet]);

  useEffect(() => {
    box.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
  }, []);

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const all = [...(box.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
    const at = all.indexOf(document.activeElement as HTMLElement);
    const to = (i: number) => {
      e.preventDefault();
      all[(i + all.length) % all.length]?.focus();
    };
    if (e.key === "ArrowDown") to(at + 1);
    else if (e.key === "ArrowUp") to(at < 0 ? all.length - 1 : at - 1);
    else if (e.key === "Home") to(0);
    else if (e.key === "End") to(all.length - 1);
    else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose(true);
    } else if (e.key === "Tab") onClose(false);
  };

  return createPortal(
    <>
      <div className={`pm-scrim${sheet ? " sheet" : ""}`} onPointerDown={() => onClose(false)} aria-hidden="true" />
      <div
        ref={box}
        className={`pm row-menu${sheet ? " sheet" : " down"}`}
        role="menu"
        aria-label={label}
        style={sheet ? undefined : (place ?? { visibility: "hidden" })}
        onKeyDown={onKey}
      >
        {sheet && <i className="pm-grab" aria-hidden="true" />}
        <div className="pm-head">{head}</div>
        <div className="pm-list">
          {items.map((it) => (
            <button
              key={it.label}
              type="button"
              role="menuitem"
              tabIndex={-1}
              className="pm-item"
              onClick={() => {
                onClose(false);
                it.onPick();
              }}
            >
              <span className="pm-ico">
                <Icon name={it.icon} small />
              </span>
              <span className="pm-text">
                <b>{it.label}</b>
                <small>{it.sub}</small>
              </span>
            </button>
          ))}
        </div>
      </div>
    </>,
    document.body,
  );
}

// `log`: the format's name in the activity log, which stays German.
const FORMATS: { id: ExportFormat; log: string; label: string; sub: string }[] = [
  texts({ id: "md", log: "Markdown", label: "Markdown" }, { sub: "cards.xf.md.sub" }),
  texts({ id: "txt", log: "Text" }, { label: "cards.xf.txt", sub: "cards.xf.txt.sub" }),
  texts({ id: "jsonl", log: "SillyTavern", label: "SillyTavern" }, { sub: "cards.xf.jsonl.sub" }),
];

const FORMAT_KEY = "rp-export-format";

function lastFormat(): ExportFormat {
  try {
    const v = localStorage.getItem(FORMAT_KEY);
    return FORMATS.some((f) => f.id === v) ? (v as ExportFormat) : "md";
  } catch {
    return "md";
  }
}

type ExportProps = {
  character: CharInfo;
  chats: ChatInfo[];
  /** All in one ZIP (the selection), else one file. */
  zip: boolean;
  onClose: () => void;
  onDone: () => void;
};

/** Chooses the format and downloads: one chat as a file, a selection as one ZIP. */
function ExportDialog({ character, chats, zip, onClose, onDone }: ExportProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [format, setFormat] = useState<ExportFormat>(lastFormat);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
    // The focus starts on the chosen format.
    d?.querySelector<HTMLInputElement>("input:checked")?.focus();
  }, []);

  const run = async () => {
    setBusy(true);
    setError("");
    try {
      localStorage.setItem(FORMAT_KEY, format);
    } catch {
      // no storage: the choice is simply not remembered
    }
    const day = new Date().toISOString().slice(0, 10);
    try {
      if (zip) {
        await download(
          "/chats/export",
          { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ ids: chats.map((c) => c.id), format, tz: tzOffset() }) },
          `Chats - ${day}.zip`,
        );
      } else {
        await download(`/chats/${chats[0].id}/export?format=${format}&tz=${tzOffset()}`, {}, `${character.name} - ${day}.${format}`);
      }
      onDone();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const title = zip ? t("cards.xf.title.zip", { n: chats.length }) : t("cards.xf.title");
  return (
    <dialog
      ref={dialog}
      className="xf"
      aria-labelledby="xf-title"
      onCancel={(e) => {
        e.preventDefault();
        if (!busy) onClose();
      }}
      onClick={(e) => e.target === e.currentTarget && !busy && onClose()}
    >
      <form
        className="xf-body"
        onSubmit={(e) => {
          e.preventDefault();
          if (!busy) void run();
        }}
      >
        <i className="pm-grab xf-grab" aria-hidden="true" />
        <h2 id="xf-title">{title}</h2>
        <p className="xf-what">{zip ? t("cards.xf.what.zip", { name: character.name }) : t("cards.xf.what.one", { chat: chatName(chats[0]), name: character.name })}</p>
        <fieldset className="xf-opts">
          <legend>{t("cards.format")}</legend>
          {FORMATS.map((f) => (
            <label className="xf-opt" key={f.id} data-log-label={`Exportformat ${f.log}`}>
              <input
                type="radio"
                name="xf-format"
                value={f.id}
                checked={format === f.id}
                onChange={() => setFormat(f.id)}
                data-log-label={`Exportformat ${f.log}`}
              />
              <span className="xf-text">
                <b>{f.label}</b>
                <small>{f.sub}</small>
              </span>
              <span className="xf-ext num">.{f.id}</span>
            </label>
          ))}
        </fieldset>
        {error && (
          <p className="help bad" role="alert">
            {t("cards.exportFailed", { error })}
          </p>
        )}
        <div className="xf-acts">
          <button type="button" className="btn ghost" onClick={onClose} disabled={busy}>
            {t("cards.cancel")}
          </button>
          <button type="submit" className="btn primary" disabled={busy}>
            <Icon name="download" small />
            {busy ? t("cards.exporting") : zip ? t("cards.exportZip") : t("cards.export")}
          </button>
        </div>
      </form>
    </dialog>
  );
}

// ---------------------------------------------------------------- card editor

type TextKey = Exclude<keyof Card, "alternate_greetings" | "tags">;
// Getters, not texts: each is read in the language on screen where it is shown.
type Field = { key: TextKey; label: () => string; tech?: string; help: () => string; rows: number };
type Section = { id: string; title: () => string; text: () => string; fields: Field[] };

const SECTIONS: Section[] = [
  {
    id: "figure",
    title: () => t("cards.edit.figure"),
    text: () => t("cards.edit.figure.text"),
    fields: [
      { key: "description", label: () => t("cards.edit.description"), help: () => t("cards.edit.description.help"), rows: 9 },
      { key: "personality", label: () => t("cards.edit.personality"), help: () => t("cards.edit.personality.help"), rows: 3 },
      { key: "scenario", label: () => t("cards.edit.scenario"), help: () => t("cards.edit.scenario.help"), rows: 3 },
    ],
  },
  {
    id: "greeting",
    title: () => t("cards.edit.greeting"),
    text: () => t("cards.edit.greeting.text"),
    fields: [
      {
        key: "first_mes",
        label: () => t("cards.edit.firstMes"),
        help: () => t("cards.edit.firstMes.help", { user: "{{user}}", char: "{{char}}" }),
        rows: 8,
      },
    ],
  },
  {
    id: "examples",
    title: () => t("cards.edit.examples"),
    text: () => t("cards.edit.examples.text"),
    fields: [{ key: "mes_example", label: () => t("cards.edit.mesExample"), help: () => t("cards.edit.mesExample.help"), rows: 7 }],
  },
  {
    id: "advanced",
    title: () => t("cards.edit.advanced"),
    text: () => t("cards.edit.advanced.text"),
    fields: [
      { key: "system_prompt", label: () => t("cards.edit.system"), tech: "System Prompt", help: () => t("cards.edit.system.help"), rows: 5 },
      {
        key: "post_history_instructions",
        label: () => t("cards.edit.post"),
        tech: "Post-History Instructions",
        help: () => t("cards.edit.post.help"),
        rows: 3,
      },
    ],
  },
];

/** What is edited: the character's card, or the version one chat plays with. */
export type CardSource = { kind: "character"; id: number } | { kind: "chat"; chatId: number };

type Loaded = Character & { custom?: boolean };

type SettingsProps = {
  source: CardSource;
  /** For the character's own card: all personas (null while the library loads) and its default one. */
  personas?: Persona[] | null;
  defaultPersonaId?: number | null;
  /** For the character's own card: the library's entry (live), the active persona and a value that
   *  changes with every library update (the check of the card against the persona is fetched again). */
  live?: CharInfo | null;
  activePersonaId?: number | null;
  rev?: unknown;
  /** Unsaved edits; they live in the app so a look at another tab does not lose them. */
  draft: Card | undefined;
  onDraft: (card: Card | null) => void;
  onLoaded: (info: CharInfo) => void;
  onDelete: (info: CharInfo) => void;
  onMissing: () => void;
};

const same = (a: Card, b: Card) => JSON.stringify(a) === JSON.stringify(b);
export const samePos = (a: [number, number], b: [number, number]) => a[0] === b[0] && a[1] === b[1];

export function CharSettings({ source, personas, defaultPersonaId = null, live, activePersonaId = null, rev, draft, onDraft, onLoaded, onDelete, onMissing }: SettingsProps) {
  const forChat = source.kind === "chat";
  const path = forChat ? `/chats/${source.chatId}/card` : `/characters/${source.id}`;
  const [saved, setSaved] = useState<Loaded | null>(null);
  const [error, setError] = useState("");
  const [state, setState] = useState<"idle" | "saving" | "saved">("idle");
  // Picture changes are staged like the text: nothing happens until "Speichern".
  const [picked, setPicked] = useState<Picked | null>(null);
  const [resetPicture, setResetPicture] = useState(false);
  const [pos, setPos] = useState<[number, number] | null>(null);
  // The chat's own version that was just removed, for one step of undo.
  const [removed, setRemoved] = useState<Card | null>(null);
  // The own short text under the card in the library, staged like the rest; null = not touched.
  const [tagline, setTagline] = useState<string | null>(null);
  // The quick actions can point at a place here (e.g. the default persona) once the card is loaded.
  useSpot(`${path}:${saved !== null}:${!!personas}`);

  useEffect(() => {
    let stale = false;
    setSaved(null);
    setError("");
    setState("idle");
    api<Loaded>(path)
      .then((c) => {
        if (stale) return;
        setSaved(c);
        onLoaded(c.info);
      })
      .catch((e) => {
        if (stale) return;
        if (e instanceof ApiError && (e.code === "character_missing" || e.code === "chat_missing")) onMissing();
        else setError(errorText(e));
      });
    return () => {
      stale = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path]);

  useEffect(() => () => void (picked && URL.revokeObjectURL(picked.url)), [picked]);

  if (!saved) {
    return (
      <div className="view">
        <div className="page set">
          {error ? (
            <div className="well result" role="alert">
              <p>{t("cards.edit.loadFailed", { error })}</p>
            </div>
          ) : (
            <div className="set-row" aria-busy="true">
              <div className="set-label">
                <span className="bar-ph" />
              </div>
              <div className="set-panel">
                <span className="bar-ph" />
                <span className="bar-ph short" />
              </div>
            </div>
          )}
        </div>
      </div>
    );
  }

  const info = saved.info;
  const card = draft ?? saved.card;
  const shownPos = pos ?? info.avatar_pos;
  const textDirty = !!draft && !same(draft, saved.card);
  const posDirty = !!pos && !samePos(pos, info.avatar_pos);
  const shownTagline = tagline ?? info.tagline ?? "";
  const taglineDirty = !forChat && tagline !== null && tagline.trim() !== (info.tagline ?? "");
  const dirty = textDirty || !!picked || resetPicture || posDirty || taglineDirty;
  const edit = (patch: Partial<Card>) => {
    const next = { ...card, ...patch };
    onDraft(same(next, saved.card) ? null : next);
    setState("idle");
  };
  const discard = () => {
    onDraft(null);
    setPicked(null);
    setResetPicture(false);
    setPos(null);
    setTagline(null);
    setError("");
  };
  const reload = async () => {
    const fresh = await api<Loaded>(path);
    setSaved(fresh);
    onLoaded(fresh.info);
  };
  const run = async (work: () => Promise<void>) => {
    setState("saving");
    setError("");
    try {
      await work();
      await reload();
      setState("saved");
    } catch (e) {
      setError(errorText(e));
      setState("idle");
    }
  };
  const save = () =>
    run(async () => {
      // The short text goes with the card (an empty one = automatic again).
      if (textDirty || taglineDirty) await postJson(path, taglineDirty ? { ...card, tagline: (tagline ?? "").trim() || null } : card, "PUT");
      setTagline(null);
      if (!forChat) {
        const base = `/characters/${source.id}`;
        if (picked) await api(`${base}/avatar`, { method: "PUT", body: picked.file });
        else if (resetPicture) await api(`${base}/avatar`, { method: "DELETE" });
        if (posDirty) await postJson(`${base}/avatar-position`, { x: shownPos[0], y: shownPos[1] }, "PUT");
      }
      onDraft(null);
      setPicked(null);
      setResetPicture(false);
      setPos(null);
      setRemoved(null);
    });
  /** The chat goes back to the character's card; the removed version can be restored once. */
  const removeOwn = () => {
    const old = saved.card;
    void run(async () => {
      await api(path, { method: "DELETE" });
      onDraft(null);
      setRemoved(old);
    });
  };
  const restoreOwn = () => {
    const old = removed;
    if (!old) return;
    void run(async () => {
      await postJson(path, old, "PUT");
      setRemoved(null);
    });
  };

  const shownPicture = resetPicture ? { ...info, custom_avatar: false, has_avatar: false } : info;
  const hasPicture = !!picked || shownPicture.has_avatar;
  const greetings = card.alternate_greetings;

  return (
    <div className="view">
      <form
        className="page set"
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty && state !== "saving") void save();
        }}
      >
        {forChat && (
          <div className="well own-note">
            <div>
              <b>{saved.custom ? t("cards.edit.own.custom") : t("cards.edit.own.only")}</b>{" "}
              {saved.custom ? t("cards.edit.own.custom.note") : t("cards.edit.own.only.note")}
            </div>
            {saved.custom && (
              <button type="button" className="btn sm" onClick={removeOwn} disabled={state === "saving"}>
                {t("cards.edit.own.back")}
              </button>
            )}
            {!saved.custom && removed && (
              <button type="button" className="btn sm" onClick={restoreOwn} disabled={state === "saving"}>
                {t("cards.edit.own.restore")}
              </button>
            )}
          </div>
        )}

        {!forChat && (
          <section className="set-row">
            <div className="set-label">
              <h2>{t("cards.edit.profile")}</h2>
              <p>{t("cards.edit.profile.text")}</p>
            </div>
            <div className="set-panel">
              <PictureField
                who={shownPicture}
                picked={picked}
                pos={shownPos}
                onPos={setPos}
                hasPicture={hasPicture}
                onPick={(p) => {
                  setPicked(p);
                  setResetPicture(false);
                  setPos(p.pos);
                  setState("idle");
                }}
                onDiscard={() => {
                  setPicked(null);
                  setPos(null);
                }}
                removeLabel={info.custom_avatar && !resetPicture ? t("cards.edit.picture.remove") : null}
                onRemove={() => setResetPicture(true)}
                cropHelp={t("cards.edit.picture.crop")}
                spot="char-picture"
                previews={
                  <>
                    <Portrait who={shownPicture} className="pv-card" preview={picked?.url} pos={shownPos} full />
                    <Portrait who={shownPicture} className="pv-round" preview={picked?.url} pos={shownPos} full />
                  </>
                }
              />
              <label className="field">
                <span className="field-label">{t("cards.edit.name")}</span>
                <input className="input" value={card.name} maxLength={120} required onChange={(e) => edit({ name: e.target.value })} />
              </label>
              <label className="field">
                <span className="field-label">{t("cards.edit.blurb")}</span>
                <span className="help">{t("cards.edit.blurb.help")}</span>
                <textarea className="textarea" rows={2} value={card.creator_notes} onChange={(e) => edit({ creator_notes: e.target.value })} />
              </label>
              <div className="field" data-spot="char-tagline">
                <span className="field-label">{t("cards.edit.tagline")}</span>
                <span className="help">{t("cards.edit.tagline.help")}</span>
                <div className="lib-tagline">
                  <input
                    className="input"
                    value={shownTagline}
                    maxLength={160}
                    placeholder={info.blurb || t("cards.edit.tagline.none")}
                    aria-label={t("cards.edit.tagline")}
                    onChange={(e) => {
                      setTagline(e.target.value);
                      setState("idle");
                    }}
                  />
                  <button type="button" className="btn" disabled={!shownTagline} onClick={() => setTagline("")} title={t("cards.edit.tagline.auto.title")}>
                    {t("cards.edit.tagline.auto")}
                  </button>
                  <span className="help lib-tagline-count num">{[...shownTagline].length} / 160</span>
                </div>
              </div>
              {live && <OwnTags character={live} />}
              <label className="field">
                <span className="field-label">{t("cards.cardTags")}</span>
                <span className="help">{t("cards.edit.cardTags.help")}</span>
                <input
                  className="input"
                  value={card.tags.join(", ")}
                  onChange={(e) => edit({ tags: e.target.value.split(",").map((tag) => tag.trimStart()) })}
                  onBlur={() => edit({ tags: card.tags.map((tag) => tag.trim()).filter(Boolean) })}
                />
              </label>
            </div>
          </section>
        )}

        {!forChat && personas && <DefaultPersona characterId={info.id} characterName={info.name} personas={personas} value={defaultPersonaId} />}

        {!forChat && personas && live && <UserFit character={live} personas={personas} activeId={activePersonaId} rev={rev} />}

        {SECTIONS.map((section) => (
          <section className="set-row" key={section.id}>
            <div className="set-label">
              <h2>{section.title()}</h2>
              <p>{section.text()}</p>
            </div>
            <div className="set-panel">
              {section.fields.map((f) => (
                <label className="field" key={f.key}>
                  <span className="field-label">
                    {f.label()}
                    {f.tech && <small> {f.tech}</small>}
                  </span>
                  <span className="help">{f.help()}</span>
                  <textarea className="textarea" rows={f.rows} value={card[f.key]} onChange={(e) => edit({ [f.key]: e.target.value })} />
                </label>
              ))}
              {section.id === "greeting" && !forChat && (
                <div className="field">
                  <span className="field-label">{t("cards.edit.greetings")}</span>
                  <span className="help">{t("cards.edit.greetings.help")}</span>
                  {greetings.map((g, i) => (
                    <div className="greet" key={i}>
                      <textarea
                        className="textarea"
                        rows={4}
                        value={g}
                        aria-label={t("cards.edit.greetings.one", { n: i + 1 })}
                        onChange={(e) => edit({ alternate_greetings: greetings.map((x, j) => (j === i ? e.target.value : x)) })}
                      />
                      <button
                        type="button"
                        className="icon-btn"
                        aria-label={t("cards.edit.greetings.remove", { n: i + 1 })}
                        onClick={() => edit({ alternate_greetings: greetings.filter((_, j) => j !== i) })}
                      >
                        <Icon name="trash" small />
                      </button>
                    </div>
                  ))}
                  <div className="btn-row">
                    <button type="button" className="btn sm" onClick={() => edit({ alternate_greetings: [...greetings, ""] })}>
                      <Icon name="plus" small />
                      {t("cards.edit.greetings.add")}
                    </button>
                  </div>
                </div>
              )}
            </div>
          </section>
        ))}

        {!forChat && (
          <section className="set-row">
            <div className="set-label">
              <h2>{t("cards.edit.delete")}</h2>
              <p>{t("cards.edit.delete.text")}</p>
            </div>
            <div className="set-panel">
              <p className="help">{t("cards.edit.delete.help")}</p>
              <div className="btn-row">
                <button type="button" className="btn sm danger" onClick={() => onDelete(info)}>
                  <Icon name="trash" small />
                  {t("cards.deleteName", { name: info.name })}
                </button>
              </div>
            </div>
          </section>
        )}

        <div className={`savebar${dirty ? " dirty" : ""}`}>
          <span className={error ? "bad" : "muted"} role="status">
            {error
              ? t("cards.save.failed", { error })
              : state === "saving"
                ? t("cards.save.saving")
                : dirty
                  ? t("cards.unsaved")
                  : state === "saved"
                    ? t("cards.save.saved")
                    : t("cards.save.none")}
          </span>
          <button type="button" className="btn ghost" onClick={discard} disabled={!dirty || state === "saving"}>
            {t("cards.save.discard")}
          </button>
          <button type="submit" className="btn primary" disabled={!dirty || state === "saving" || !card.name.trim()}>
            {t("cards.save.save")}
          </button>
        </div>
      </form>
    </div>
  );
}
