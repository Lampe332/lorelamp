// The picture block of the card editor and the persona editor: the picture with its crop, and every way
// to set one (file, drop on the picture, Ctrl+V, a link). Nothing is saved here; the editor stages the
// picked picture until "Speichern".

import { useRef, useState, type ClipboardEvent, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import { useFormFactor } from "./app/formfactor";
import { t } from "./i18n";
import { asLink, useImageIntake, type Picked } from "./image";
import { Icon, Portrait } from "./ui";

type Who = Parameters<typeof Portrait>[0]["who"];

type CropProps = {
  who: Who;
  preview?: string;
  pos: [number, number];
  onPos: (pos: [number, number]) => void;
  /** A persona's picture instead of a character's. */
  kind?: "character" | "persona";
};

const clampPct = (v: number) => Math.round(Math.min(100, Math.max(0, v)) * 10) / 10;

/** The square crop of the picture; dragging it (or the arrow keys) moves the visible part. */
export function CropBox({ who, preview, pos, onPos, kind }: CropProps) {
  const drag = useRef<{ x: number; y: number; pos: [number, number]; spanX: number; spanY: number } | null>(null);

  const down = (e: PointerEvent<HTMLDivElement>) => {
    const box = e.currentTarget;
    const img = box.querySelector("img");
    if (!img || !img.naturalWidth) return;
    // How far the picture overhangs the frame decides how far one pixel of dragging moves it.
    const scale = Math.max(box.clientWidth / img.naturalWidth, box.clientHeight / img.naturalHeight);
    const overX = img.naturalWidth * scale - box.clientWidth;
    const overY = img.naturalHeight * scale - box.clientHeight;
    // No overhang here (a square picture) can still be one in the wider card frame: move by frame size then.
    drag.current = { x: e.clientX, y: e.clientY, pos, spanX: overX > 1 ? overX : box.clientWidth, spanY: overY > 1 ? overY : box.clientHeight };
    box.setPointerCapture(e.pointerId);
  };
  const move = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    onPos([clampPct(d.pos[0] - ((e.clientX - d.x) / d.spanX) * 100), clampPct(d.pos[1] - ((e.clientY - d.y) / d.spanY) * 100)]);
  };
  const key = (e: KeyboardEvent) => {
    const step: Record<string, [number, number]> = { ArrowLeft: [-2, 0], ArrowRight: [2, 0], ArrowUp: [0, -2], ArrowDown: [0, 2] };
    const by = step[e.key];
    if (!by) return;
    e.preventDefault();
    onPos([clampPct(pos[0] + by[0]), clampPct(pos[1] + by[1])]);
  };

  return (
    <div
      className="crop"
      tabIndex={0}
      role="group"
      aria-label={t("cards.pic.crop")}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={() => (drag.current = null)}
      onPointerCancel={() => (drag.current = null)}
      onKeyDown={key}
    >
      <Portrait who={who} className="pic" preview={preview} pos={pos} kind={kind} full />
    </div>
  );
}

type FieldProps = {
  who: Who;
  kind?: "character" | "persona";
  /** The picture picked but not saved yet. */
  picked: Picked | null;
  /** The crop shown now (saved or chosen). */
  pos: [number, number];
  onPos: (pos: [number, number]) => void;
  hasPicture: boolean;
  onPick: (p: Picked) => void;
  onDiscard: () => void;
  /** Label of the button that removes the stored picture; null while there is nothing to remove. */
  removeLabel: string | null;
  onRemove: () => void;
  /** Help line with a picture (how the crop works). */
  cropHelp: string;
  /** Where Ctrl+K "Bild setzen" points. */
  spot: string;
  /** The small views of the crop (card, round). */
  previews: ReactNode;
};

export function PictureField({ who, kind, picked, pos, onPos, hasPicture, onPick, onDiscard, removeLabel, onRemove, cropHelp, spot, previews }: FieldProps) {
  const picker = useRef<HTMLInputElement>(null);
  const form = useFormFactor();
  const [linking, setLinking] = useState(false);
  const [link, setLink] = useState("");
  const take = (p: Picked) => {
    onPick(p);
    setLinking(false);
    setLink("");
  };
  const intake = useImageIntake(take);
  const linkOk = asLink(link) !== null;

  const loadLink = (value = link) => {
    const href = asLink(value);
    if (href) intake.fromLink(href);
  };
  const onLinkPaste = (e: ClipboardEvent<HTMLInputElement>) => {
    const href = asLink(e.clipboardData.getData("text/plain"));
    if (!href) return;
    // A pasted link loads at once: one step less.
    e.preventDefault();
    setLink(href);
    loadLink(href);
  };

  return (
    <div className={`pic-row${intake.over ? " drop-over" : ""}`} data-spot={spot} {...intake.dropProps}>
      <div className="pic-drop">
        {hasPicture ? (
          <CropBox who={who} preview={picked?.url} pos={pos} onPos={onPos} kind={kind} />
        ) : (
          <Portrait who={who} className="pic" kind={kind} full />
        )}
        <span className="pic-drop-hint" aria-hidden="true">
          <Icon name="upload" />
          {t("cards.pic.drop")}
        </span>
      </div>
      <div className="pic-actions">
        <span className="field-label">{t("cards.pic.label")}</span>
        <p className="help">
          {hasPicture ? cropHelp : t("cards.pic.types")} {form === "desktop" ? t("cards.pic.new.desktop") : t("cards.pic.new.touch")}
        </p>
        <div className="btn-row">
          <button type="button" className="btn sm" onClick={() => picker.current?.click()} data-spot-focus>
            <Icon name="upload" small />
            {t("cards.pic.choose")}
          </button>
          <button type="button" className="btn sm" aria-expanded={linking} onClick={() => setLinking((v) => !v)}>
            <Icon name="link" small />
            {t("cards.pic.link")}
          </button>
          {picked && (
            <button type="button" className="btn sm ghost" onClick={onDiscard}>
              {t("cards.pic.discard")}
            </button>
          )}
          {!picked && removeLabel && (
            <button type="button" className="btn sm ghost" onClick={onRemove}>
              {removeLabel}
            </button>
          )}
        </div>
        {linking && (
          <div className="pic-link" role="group" aria-label={t("cards.pic.link.group")}>
            <input
              className="input"
              type="url"
              inputMode="url"
              autoFocus
              placeholder={t("cards.pic.link.placeholder")}
              aria-label={t("cards.pic.link.label")}
              value={link}
              onChange={(e) => {
                setLink(e.target.value);
                intake.clearError();
              }}
              onPaste={onLinkPaste}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault(); // not the editor's submit
                  loadLink();
                } else if (e.key === "Escape") {
                  e.stopPropagation();
                  setLinking(false);
                }
              }}
            />
            <button type="button" className="btn sm" disabled={!linkOk || intake.busy} onClick={() => loadLink()}>
              {intake.busy ? t("cards.pic.link.loading") : t("cards.pic.link.load")}
            </button>
          </div>
        )}
        {intake.busy && !linking && (
          <p className="help" role="status">
            {t("cards.pic.loading")}
          </p>
        )}
        {intake.error && (
          <p className="help bad" role="alert">
            {intake.error}
          </p>
        )}
        <input
          ref={picker}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          hidden
          onChange={(e) => {
            const file = e.target.files?.[0];
            e.target.value = "";
            if (file) intake.fromFile(file);
          }}
        />
      </div>
      {hasPicture && (
        <div className="pic-previews" aria-hidden="true">
          {previews}
        </div>
      )}
    </div>
  );
}
