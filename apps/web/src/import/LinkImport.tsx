// The link part of the import page. Pasted or typed links (one or many, see `links.ts`) first load
// a preview of each card — the server fetches it and keeps it in memory for 15 minutes — and only
// "Importieren" stores it. A site that allows no fetch by a program, blocks it or shows no card
// gets the way through the user: open the page, download the file, drop or paste it here.

import { useRef, useState, type ClipboardEvent as ReactClipboardEvent, type FormEvent, type KeyboardEvent } from "react";
import { linkPreviewImageUrl, type Library } from "../api";
import { t } from "../i18n";
import { rich } from "../i18n/rich";
import { Icon, Portrait } from "../ui";
import "./link.css";
import {
  cardJson,
  clearLinks,
  confirmAllLinks,
  confirmLink,
  discardLink,
  linkReason,
  linksIn,
  reloadLink,
  setDraft,
  takeLinks,
  useDraft,
  useLinks,
  type LinkAnswer,
  type LinkItem,
  type Preview as PreviewData,
} from "./links";

/** Names of the sites as they write them. */
const SOURCES: Record<string, string> = {
  chub: "Chub",
  risurealm: "RisuRealm",
  janitorai: "JanitorAI",
  jannyai: "JannyAI",
  character_tavern: "Character Tavern",
  hammerai: "HammerAI",
  pygmalion: "Pygmalion",
  aicharactercards: "AICharacterCards",
  perchance: "Perchance",
  wyvern: "WyvernChat",
  backyard: "Backyard AI",
  charavault: "CharaVault",
};
const host = (url: string | undefined) => {
  try {
    return new URL(url ?? "").hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
};
const sourceName = (a: { source: string; page_url?: string }) => SOURCES[a.source] ?? (host(a.page_url) || t("system.link.thisSite"));

/** Why a site gets the way by file: headline and the reason in a sentence. */
function fileOnlyText(answer: LinkAnswer): { head: string; why: string } {
  const site = sourceName(answer);
  switch (answer.reason_code) {
    case "no_card_found":
      return { head: t("system.link.fileOnly.noCard", { site }), why: t("system.link.fileOnly.noCard.why") };
    case "bot_protection":
      return { head: t("system.link.fileOnly.head", { site }), why: t("system.link.fileOnly.bot") };
    case "definition_private":
      return { head: t("system.link.fileOnly.head", { site }), why: t("system.link.fileOnly.private") };
    case "terms_forbid_automation":
      return { head: t("system.link.fileOnly.head", { site }), why: t("system.link.fileOnly.terms") };
    default:
      return { head: t("system.link.fileOnly.head", { site }), why: t("system.link.fileOnly.other") };
  }
}

/** Card texts say `{{char}}` for the figure; the preview reads better with its name. */
const named = (text: string, name: string) => text.replace(/\{\{char\}\}/gi, name).trim();

/** Most tags shown; the rest is counted. */
const MAX_TAGS = 12;
const MAX_TAGS_COMPACT = 4;

type PreviewProps = {
  preview: PreviewData;
  importing: boolean;
  error?: string;
  library: Library | null;
  compact: boolean;
  headingId: string;
  onImport: () => void;
  onDiscard: () => void;
};

function Preview({ preview, importing, error, library, compact, headingId, onImport, onDiscard }: PreviewProps) {
  const { card } = preview;
  const name = card.name.trim() || t("system.link.noName");
  const dup = preview.duplicate_of !== null ? library?.characters.find((c) => c.id === preview.duplicate_of) : undefined;
  const description = named(card.description || card.creator_notes, name);
  const first = named(card.first_mes, name);
  const maxTags = compact ? MAX_TAGS_COMPACT : MAX_TAGS;
  const more = preview.more_cards ?? 0;
  return (
    <article className={compact ? "imp-pv lnk-compact" : "imp-pv"} aria-labelledby={headingId}>
      <Portrait
        className="pic imp-pv-pic"
        who={{ id: 0, name, has_avatar: false, avatar_rev: 0, avatar_pos: [50, 30] }}
        preview={preview.has_image ? linkPreviewImageUrl(preview.preview_id) : undefined}
      />
      <div className="imp-pv-head">
        <h3 className="imp-pv-name" id={headingId}>
          {name}
        </h3>
        <p className="imp-pv-meta">
          {card.creator.trim() && (
            <span>{rich("system.link.by", { creator: <b>{card.creator.trim()}</b> })}</span>
          )}
          <a href={preview.page_url} target="_blank" rel="noopener noreferrer">
            {t("system.link.on", { site: sourceName(preview) })}
            <Icon name="link" small />
          </a>
        </p>
      </div>
      <div className="imp-pv-body">
        {preview.duplicate_of !== null && (
          <p className="imp-pv-dup">
            <Icon name="copy" small />
            <span>
              {t("system.link.dup")}
              {dup ? (
                <>
                  : <a href={`#/char/${dup.id}/settings`}>{dup.name}</a>
                </>
              ) : (
                <>
                  {" "}
                  · <a href={`#/char/${preview.duplicate_of}/settings`}>{t("system.link.openFigure")}</a>
                </>
              )}
            </span>
          </p>
        )}
        {card.tags.length > 0 && (
          <ul className="imp-pv-tags" aria-label={t("system.link.tags")}>
            {card.tags.slice(0, maxTags).map((tag) => (
              <li key={tag} className="tag">
                {tag}
              </li>
            ))}
            {card.tags.length > maxTags && <li className="tag">+{card.tags.length - maxTags}</li>}
          </ul>
        )}
        {description && <p className="imp-pv-desc">{description}</p>}
        {more > 0 && <p className="lnk-more">{t("system.link.moreInZip", { n: more })}</p>}
        {first && !compact && (
          <details className="imp-pv-first">
            <summary>
              <Icon name="chevron" small />
              {t("system.link.firstMessage")}
            </summary>
            <p>{first}</p>
          </details>
        )}
        {error && (
          <p className="imp-link-msg bad" role="alert">
            <Icon name="alert" small />
            <span>{error}</span>
          </p>
        )}
        <div className="btn-row imp-pv-actions">
          <button className="btn primary" onClick={onImport} disabled={importing}>
            <Icon name="download" small />
            {importing ? t("system.link.importing") : more > 0 ? t("system.link.importAll", { n: more + 1 }) : t("system.link.import")}
          </button>
          <button className="btn ghost" onClick={onDiscard} disabled={importing}>
            {t("system.link.discard")}
          </button>
        </div>
      </div>
    </article>
  );
}

type EntryProps = { item: LinkItem; library: Library | null; touch: boolean; compact: boolean; onPickFile: () => void; onDiscard: (key: number) => void; onImported: () => void };

/** One link and what became of it. `compact`: one of several in the list. */
function Entry({ item, library, touch, compact, onPickFile, onDiscard, onImported }: EntryProps) {
  const { state } = item;
  const where = host(item.url) || item.url;
  // In the list every line says which link it is about.
  const lead = compact ? <b className="lnk-where">{where}</b> : null;
  const remove = compact ? (
    <button className="btn sm ghost lnk-x" onClick={() => onDiscard(item.key)} aria-label={t("system.link.removeOne", { where })}>
      <Icon name="close" small />
    </button>
  ) : null;

  if (state.kind === "waiting" || state.kind === "loading") {
    return (
      <p className={state.kind === "waiting" ? "imp-wait lnk-idle" : "imp-wait"} role="status">
        {compact ? (
          <span>
            {lead} {state.kind === "waiting" ? t("system.link.waiting") : t("system.link.loading")}
          </span>
        ) : (
          t("system.link.loadingOne")
        )}
      </p>
    );
  }
  if (state.kind === "preview") {
    return (
      <Preview
        preview={state.preview}
        importing={state.importing}
        error={state.error}
        library={library}
        compact={compact}
        headingId={compact ? `imp-pv-name-${item.key}` : "imp-pv-name"}
        onImport={() => void confirmLink(item.key).then(onImported)}
        onDiscard={() => onDiscard(item.key)}
      />
    );
  }
  if (state.kind === "error") {
    return (
      <div className="imp-link-msg bad" role="alert">
        <Icon name="alert" small />
        <span>
          {lead} {state.text}{" "}
          <button className="btn sm solid imp-inline" onClick={() => reloadLink(item.key)}>
            {t("system.link.again")}
          </button>
        </span>
        {remove}
      </div>
    );
  }
  if (state.kind === "expired") {
    return (
      <div className="imp-link-msg bad" role="alert">
        <Icon name="alert" small />
        <span>
          {lead} {t("system.link.expired")}{" "}
          <button className="btn sm solid imp-inline" onClick={() => reloadLink(item.key)}>
            {t("system.link.reload")}
          </button>
        </span>
        {remove}
      </div>
    );
  }

  const { answer } = state;
  const who = answer.character_id ? library?.characters.find((c) => c.id === answer.character_id) : undefined;
  if (answer.status === "imported" || answer.status === "duplicate") {
    const also = answer.more ? answer.more.imported : 0;
    return (
      <p className="imp-link-msg">
        <Icon name={answer.status === "imported" ? "check" : "copy"} small />
        <span>
          {who
            ? rich(answer.status === "imported" ? "system.link.importedAs" : "system.link.dupAs", { name: <a href={`#/char/${who.id}/settings`}>{who.name}</a> })
            : answer.status === "imported"
              ? t("system.link.imported")
              : t("system.link.dupShort")}
          {also > 0 && ` ${t("system.link.andMore", { n: also })}`}
        </span>
        {remove}
      </p>
    );
  }
  if (answer.status === "error") {
    return (
      <p className="imp-link-msg bad" role="alert">
        <Icon name="alert" small />
        <span>
          {lead} {linkReason(answer.reason_code) ?? t("system.link.noCardFrom", { site: sourceName(answer) })}
        </span>
        {remove}
      </p>
    );
  }
  const text = fileOnlyText(answer);
  return (
    <div className={compact ? "imp-fileonly lnk-fileonly" : "imp-fileonly"} role="status">
      <b>{text.head}</b>
      <p>
        {text.why}{" "}
        {touch ? t("system.link.fileOnly.doTouch") : t("system.link.fileOnly.do")}
      </p>
      <div className="btn-row">
        {answer.page_url && (
          <a className="btn primary" href={answer.page_url} target="_blank" rel="noopener noreferrer">
            {t("system.link.openPage")}
          </a>
        )}
        <button className="btn solid" onClick={onPickFile}>
          {t("system.link.pickFile")}
        </button>
        {compact && (
          <button className="btn ghost" onClick={() => onDiscard(item.key)}>
            {t("system.link.remove")}
          </button>
        )}
      </div>
    </div>
  );
}

type Props = { library: Library | null; touch: boolean; onPickFile: () => void; onCardText: (text: string) => void };

export function LinkImport({ library, touch, onPickFile, onCardText }: Props) {
  const items = useLinks();
  const url = useDraft();
  const [fieldError, setFieldError] = useState("");
  const [note, setNote] = useState("");
  const field = useRef<HTMLTextAreaElement>(null);
  const many = items.length > 1;
  const busy = items.filter((i) => i.state.kind === "waiting" || i.state.kind === "loading").length;
  const ready = items.filter((i) => i.state.kind === "preview" && i.state.preview.duplicate_of === null);
  const importing = items.some((i) => i.state.kind === "preview" && i.state.importing);

  const take = (links: string[]) => {
    setFieldError("");
    setNote(takeLinks(links));
  };

  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    const links = linksIn(url);
    if (links.length === 0) {
      setFieldError(t("system.link.notLink"));
      return;
    }
    take(links);
  };
  // Enter loads the preview; Shift+Enter makes room for another link.
  const keyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) submit(e);
  };
  // Links pasted into the empty field show their previews at once (nothing is imported yet); a
  // pasted card text goes to the file import.
  const paste = (e: ReactClipboardEvent<HTMLTextAreaElement>) => {
    const text = e.clipboardData.getData("text");
    if (url.trim() !== "") return;
    if (cardJson(text)) {
      e.preventDefault();
      onCardText(text);
      return;
    }
    const links = linksIn(text);
    if (links.length > 0) {
      e.preventDefault();
      take(links);
    }
  };

  const discard = (key: number) => {
    discardLink(key);
    if (!many) {
      setDraft("");
      field.current?.focus();
    }
  };
  const clear = () => {
    clearLinks();
    setDraft("");
    setNote("");
    field.current?.focus();
  };
  // A single imported link leaves the field empty for the next one.
  const imported = () => {
    if (!many) setDraft("");
  };

  const lines = Math.min(6, Math.max(1, url.split("\n").length));
  const shared = { library, touch, onPickFile, onDiscard: discard, onImported: imported };
  return (
    <div className="imp-link" data-spot="import-link">
      <form className="imp-link-form" onSubmit={submit} noValidate>
        <label htmlFor="imp-link-url">{t("system.link.paste")}</label>
        <div className="imp-link-row">
          <textarea
            ref={field}
            id="imp-link-url"
            className="input lnk-field"
            rows={lines}
            inputMode="url"
            enterKeyHint="go"
            autoComplete="off"
            autoCapitalize="off"
            spellCheck={false}
            placeholder="https://chub.ai/characters/…"
            value={url}
            onChange={(e) => {
              setDraft(e.target.value);
              setFieldError("");
            }}
            onKeyDown={keyDown}
            onPaste={paste}
          />
          <button className="btn solid" type="submit" disabled={url.trim() === ""}>
            {t("system.link.preview")}
          </button>
        </div>
      </form>
      <p className="imp-note">
        {touch ? (
          t("system.link.noteTouch")
        ) : (
          rich("system.link.note", {
            keys: (
              <>
                <kbd>{t("shell.key.ctrl")}</kbd> <kbd>V</kbd>
              </>
            ),
          })
        )}
      </p>
      {fieldError && (
        <p className="imp-link-msg bad" role="alert">
          <Icon name="alert" small />
          <span>{fieldError}</span>
        </p>
      )}
      {note && (
        <p className="imp-link-msg bad" role="alert">
          <Icon name="alert" small />
          <span>{note}</span>
        </p>
      )}
      <div aria-live="polite">
        {many && (
          <div className="lnk-bar">
            <p className="lnk-count num" id="lnk-count">
              <b>{t("system.link.count", { n: items.length })}</b>
              {busy > 0 ? (
                <span>{t("system.link.fetched", { done: items.length - busy, total: items.length })}</span>
              ) : (
                <span>{t("system.link.allFetched")}</span>
              )}
            </p>
            {busy > 0 && (
              <div className="imp-bar" role="progressbar" aria-labelledby="lnk-count" aria-valuemin={0} aria-valuemax={items.length} aria-valuenow={items.length - busy}>
                <i style={{ transform: `scaleX(${(items.length - busy) / items.length})` }} />
              </div>
            )}
            <div className="btn-row">
              {ready.length > 1 && (
                <button className="btn primary" onClick={() => void confirmAllLinks()} disabled={importing}>
                  <Icon name="download" small />
                  {t("system.link.importAll", { n: ready.length })}
                </button>
              )}
              <button className="btn ghost" onClick={clear} disabled={importing}>
                {t("system.import.clear")}
              </button>
            </div>
          </div>
        )}
        {many ? (
          <ul className="lnk-list" aria-label={t("system.link.list")}>
            {items.map((item) => (
              <li key={item.key}>
                <Entry item={item} compact {...shared} />
              </li>
            ))}
          </ul>
        ) : (
          items.map((item) => <Entry key={item.key} item={item} compact={false} {...shared} />)
        )}
      </div>
    </div>
  );
}
