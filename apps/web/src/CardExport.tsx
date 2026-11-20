// Export of a character card (PNG, JSON V2, JSON V3) and "Als Persona übernehmen", in the header of the
// character's page; the chat's own version of the card from its card view.

import { useEffect, useRef, useState } from "react";
import { download, errorText, postJson } from "./api";
import { t, texts } from "./i18n";
import { useSpotAsked } from "./spot";
import { Icon } from "./ui";

export type CardFormat = "png" | "json_v2" | "json_v3";

const FORMATS: { id: CardFormat; label: string; sub: string; ext: string }[] = [
  texts({ id: "png", label: "PNG", ext: "png" }, { sub: "cards.cx.png.sub" }),
  texts({ id: "json_v2", label: "JSON V2", ext: "json" }, { sub: "cards.cx.v2.sub" }),
  texts({ id: "json_v3", label: "JSON V3", ext: "json" }, { sub: "cards.cx.v3.sub" }),
];

const FORMAT_KEY = "rp-card-format";

function lastFormat(): CardFormat {
  try {
    const v = localStorage.getItem(FORMAT_KEY);
    return FORMATS.some((f) => f.id === v) ? (v as CardFormat) : "png";
  } catch {
    return "png";
  }
}

type DialogProps = {
  characterId: number;
  name: string;
  /** The version of the card this chat plays with, instead of the character's own. */
  chatId?: number;
  /** The character has a picture of its own (else the PNG carries a grey placeholder). */
  hasPicture: boolean;
  onClose: () => void;
  /** After the file was saved, with its name. */
  onSaved: (file: string) => void;
};

/** Chooses the format and saves the card. */
export function CardExportDialog({ characterId, name, chatId, hasPicture, onClose, onSaved }: DialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [format, setFormat] = useState<CardFormat>(lastFormat);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
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
    const ext = FORMATS.find((f) => f.id === format)?.ext ?? "png";
    try {
      const file = await download(`/characters/${characterId}/export?format=${format}${chatId ? `&chat=${chatId}` : ""}`, {}, `${name}.${ext}`);
      // Cancelled save dialog: stay in this dialog, nothing to report.
      if (file === null) return setBusy(false);
      onSaved(file);
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <dialog
      ref={dialog}
      className="xf"
      aria-labelledby="cx-title"
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
        <h2 id="cx-title">{chatId ? t("cards.cx.title.chat") : t("cards.cx.title")}</h2>
        <p className="xf-what">{chatId ? t("cards.cx.what.chat", { name }) : t(hasPicture ? "cards.cx.what" : "cards.cx.what.bare", { name })}</p>
        <fieldset className="xf-opts">
          <legend>{t("cards.format")}</legend>
          {FORMATS.map((f) => (
            <label className="xf-opt" key={f.id} data-log-label={`Kartenformat ${f.label}`}>
              <input type="radio" name="cx-format" value={f.id} checked={format === f.id} onChange={() => setFormat(f.id)} data-log-label={`Kartenformat ${f.label}`} />
              <span className="xf-text">
                <b>{f.label}</b>
                <small>{f.sub}</small>
              </span>
              <span className="xf-ext num">.{f.ext}</span>
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
            {busy ? t("cards.exporting") : t("cards.export")}
          </button>
        </div>
      </form>
    </dialog>
  );
}

type ToolsProps = {
  characterId: number;
  name: string;
  hasPicture: boolean;
  onToast: (text: string) => void;
};

/** The header buttons of a character's page: export the card, take it over as a persona. Ctrl+K can
 *  ask for either ("card-export", "as-persona"). */
export function CharTools({ characterId, name, hasPicture, onToast }: ToolsProps) {
  const [open, setOpen] = useState(false);
  const [taking, setTaking] = useState(false);
  const [askedExport, doneExport] = useSpotAsked("card-export");
  const [askedPersona, donePersona] = useSpotAsked("as-persona");

  const asPersona = async () => {
    setTaking(true);
    try {
      const r = await postJson<{ status: "imported" | "duplicate"; persona_id: number | null }>(`/characters/${characterId}/as-persona`, {});
      onToast(t(r.status === "duplicate" ? "cards.cx.persona.dup" : "cards.cx.persona.done", { name }));
      if (r.persona_id) location.hash = `#/personas/${r.persona_id}`;
    } catch (e) {
      onToast(t("cards.cx.persona.failed", { error: errorText(e) }));
    } finally {
      setTaking(false);
    }
  };

  useEffect(() => {
    if (askedExport) {
      doneExport();
      setOpen(true);
    }
  }, [askedExport, doneExport]);
  useEffect(() => {
    if (askedPersona) {
      donePersona();
      void asPersona();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [askedPersona]);

  return (
    <>
      <button type="button" className="btn sm" onClick={() => void asPersona()} disabled={taking} aria-label={t("cards.cx.persona")} title={t("cards.cx.persona.title")}>
        <Icon name="persona" small />
        <span className="hide-sm">{t("cards.cx.persona")}</span>
      </button>
      <button type="button" className="btn sm" onClick={() => setOpen(true)} aria-label={t("cards.exportDots")} aria-haspopup="dialog">
        <Icon name="download" small />
        <span className="hide-sm">{t("cards.exportDots")}</span>
      </button>
      {open && <CardExportDialog characterId={characterId} name={name} hasPicture={hasPicture} onClose={() => setOpen(false)} onSaved={(file) => onToast(t("cards.savedFile", { file }))} />}
    </>
  );
}

/** In the card view of one chat: the version this chat plays with. */
export function ChatCardExport({ characterId, name, chatId }: { characterId: number; name: string; chatId: number }) {
  const [open, setOpen] = useState(false);
  // No toast here: a short status next to the button.
  const [saved, setSaved] = useState("");
  useEffect(() => {
    if (!saved) return;
    const timer = window.setTimeout(() => setSaved(""), 4000);
    return () => window.clearTimeout(timer);
  }, [saved]);
  return (
    <>
      <button type="button" className="btn sm" onClick={() => setOpen(true)} aria-label={t("cards.cx.title.chat")} aria-haspopup="dialog">
        <Icon name="download" small />
        <span className="hide-sm">{t("cards.cx.title.chat")}</span>
      </button>
      {saved && (
        <span className="help" role="status">
          {t("cards.savedFile", { file: saved })}
        </span>
      )}
      {open && <CardExportDialog characterId={characterId} name={name} chatId={chatId} hasPicture onClose={() => setOpen(false)} onSaved={setSaved} />}
    </>
  );
}
