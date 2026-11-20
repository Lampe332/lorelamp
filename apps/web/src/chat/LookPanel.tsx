// "Aussehen" of one chat: its backdrop picture (choose, move, dim, soften, make it the character's
// default) and what it changes of the reading settings. A panel beside the story on a desktop, a
// sheet from below on a phone; the chat stays in view and follows every change at once.

import { useEffect, useRef, useState } from "react";
import { errorText, type CharInfo } from "../api";
import { useFormFactor } from "../app/formfactor";
import { t } from "../i18n";
import { rich } from "../i18n/rich";
import { PickError, prepare, useImageIntake } from "../image";
import { Icon } from "../ui";
import { LookFields } from "./LookFields";
import { PresetGallery } from "./PresetGallery";
import { presetName } from "./presets";
import {
  backdropToCharacter,
  deleteCharacterBackdrop,
  deleteChatBackdrop,
  putChatBackdrop,
  putChatLook,
  shownBackdrop,
  type BackdropMode,
  type ChatLook,
  type Frame,
  type Look,
} from "./look";

type Props = {
  chatId: number;
  character: CharInfo;
  view: ChatLook;
  setLook: (look: Look) => void;
  setFrame: (frame: Frame) => void;
  reload: () => void;
  /** The character's picture is shown large. */
  stage: boolean;
  onStage: (on: boolean) => void;
  onClose: () => void;
};

/** The panel while the chat's look is still on its way: its frame and the shapes of what comes, so
 *  it never opens blank. */
export function LookPanelLoading({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <aside className="cv-panel" role="dialog" aria-label={t("chat.look.open")} aria-busy="true">
      <header className="cv-panel-head">
        <h2>{t("chat.look.open")}</h2>
        <button type="button" className="icon-btn" onClick={onClose} aria-label={t("chat.panel.close")} title={t("chat.closeEsc")}>
          <Icon name="close" small />
        </button>
      </header>
      <div className="cv-panel-body cv-panel-wait">
        <span className="sr" role="status">
          {t("chat.panel.loading")}
        </span>
        <span className="bar-ph short" aria-hidden="true" />
        <span className="cv-ph-pic" aria-hidden="true" />
        <span className="bar-ph" aria-hidden="true" />
        <span className="bar-ph" aria-hidden="true" />
        <span className="bar-ph short" aria-hidden="true" />
      </div>
    </aside>
  );
}

export function LookPanel({ chatId, character, view, setLook, setFrame, reload, stage, onStage, onClose }: Props) {
  const panel = useRef<HTMLElement>(null);
  const file = useRef<HTMLInputElement>(null);
  const desktop = useFormFactor() === "desktop";
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");
  const shown = shownBackdrop(chatId, view);
  const { mode, own, character: theirs } = view.backdrop;
  const name = character.name;

  /** One change on the server, then the chat's look again; a failure is said in the panel. */
  const run = async (work: () => Promise<unknown>, done = "") => {
    setBusy(true);
    setError("");
    setNote("");
    try {
      await work();
      setNote(done);
    } catch (e) {
      setError(e instanceof PickError ? e.message : errorText(e));
    } finally {
      setBusy(false);
      reload();
    }
  };
  const upload = (blob: Blob) => run(() => putChatBackdrop(chatId, blob));
  // A drop on the panel or Ctrl+V while it is open takes a picture too.
  const intake = useImageIntake((p) => {
    URL.revokeObjectURL(p.url);
    void upload(p.file);
  });
  const setMode = (m: BackdropMode) => run(() => putChatLook(chatId, { backdrop: m }));

  useEffect(() => {
    panel.current?.focus({ preventScroll: true });
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const slider = (key: keyof Frame, label: string, max: number, unit: string) =>
    shown && (
      <div className="cv-field">
        <div className="cv-field-head">
          <label htmlFor={`cv-bg-${key}`}>{label}</label>
          <output className="num">
            {shown[key]} {unit}
          </output>
        </div>
        <input
          id={`cv-bg-${key}`}
          className="cv-range"
          type="range"
          min={0}
          max={max}
          step={1}
          value={shown[key]}
          aria-valuetext={`${shown[key]} ${unit}`}
          onChange={(e) => setFrame({ x: shown.x, y: shown.y, dim: shown.dim, blur: shown.blur, [key]: Number(e.target.value) })}
        />
      </div>
    );

  return (
    <aside className={`cv-panel${intake.over ? " over" : ""}`} role="dialog" aria-label={t("chat.look.open")} tabIndex={-1} ref={panel} {...intake.dropProps}>
      <header className="cv-panel-head">
        <h2>{t("chat.look.open")}</h2>
        <button type="button" className="icon-btn" onClick={onClose} aria-label={t("chat.panel.close")} title={t("chat.closeEsc")}>
          <Icon name="close" small />
        </button>
      </header>
      <div className="cv-panel-body">
        <section className="cv-sec" aria-label={t("chat.panel.bg")}>
          <h3>{t("chat.panel.bg")}</h3>
          {shown ? (
            <div className="cv-bg-thumb">
              <img src={shown.url} alt="" style={{ objectPosition: `${shown.x}% ${shown.y}%` }} draggable={false} />
              <span>
                {presetName(shown.preset) ? `${presetName(shown.preset)} · ` : ""}
                {shown.of === "own" ? t("chat.panel.onlyThis") : t("chat.panel.defaultOf", { name })}
              </span>
            </div>
          ) : (
            <p className="help">
              {mode === "none" && (own || theirs) ? t("chat.panel.noneShown") : desktop ? t("chat.panel.empty.desktop") : t("chat.panel.empty.touch")}
            </p>
          )}
          {(own || theirs) && (
            <div className="seg cv-seg" role="group" aria-label={t("chat.panel.which")}>
              <button type="button" aria-pressed={mode === "character"} disabled={busy || !theirs} onClick={() => void setMode("character")} title={theirs ? undefined : t("chat.panel.noDefault", { name })}>
                {t("chat.panel.fromName", { name })}
              </button>
              <button type="button" aria-pressed={mode === "own"} disabled={busy || !own} onClick={() => void setMode("own")} title={own ? undefined : t("chat.panel.noOwn")}>
                {t("chat.panel.own")}
              </button>
              <button type="button" aria-pressed={mode === "none" || (mode === "character" && !theirs)} disabled={busy} onClick={() => void setMode("none")}>
                {t("chat.panel.none")}
              </button>
            </div>
          )}
          <PresetGallery chatId={chatId} name={name} view={view} shownPreset={shown?.preset} showsNone={!shown} busy={busy} run={run} />
          <div className="btn-row">
            <input
              ref={file}
              type="file"
              accept="image/png,image/jpeg,image/webp,image/gif"
              className="sr"
              tabIndex={-1}
              aria-label={t("chat.panel.uploadLabel")}
              onChange={(e) => {
                const f = e.target.files?.[0];
                e.target.value = "";
                if (f)
                  void run(async () => {
                    const p = await prepare(f, f.name);
                    URL.revokeObjectURL(p.url);
                    await putChatBackdrop(chatId, p.file);
                  });
              }}
            />
            <button type="button" className="btn solid" disabled={busy || intake.busy} onClick={() => file.current?.click()}>
              <Icon name="upload" small />
              {own ? t("chat.panel.replace") : t("chat.panel.upload")}
            </button>
            {shown?.of === "own" && (
              <button type="button" className="btn ghost" disabled={busy} onClick={() => void run(() => deleteChatBackdrop(chatId))}>
                {t("chat.panel.remove")}
              </button>
            )}
          </div>
          {slider("dim", t("chat.panel.dim"), 90, "%")}
          {slider("blur", t("chat.panel.blur"), 24, "px")}
          {slider("x", t("chat.panel.x"), 100, "%")}
          {slider("y", t("chat.panel.y"), 100, "%")}
          {shown?.of === "own" && (
            <button type="button" className="btn cv-wide" disabled={busy} onClick={() => void run(() => backdropToCharacter(chatId), t("chat.panel.toDefault.done", { name }))}>
              {t("chat.panel.toDefault", { name })}
            </button>
          )}
          {shown?.of === "character" && (
            <>
              <p className="help">{t("chat.panel.theirs", { name })}</p>
              <button type="button" className="btn cv-wide" disabled={busy} onClick={() => void run(() => deleteCharacterBackdrop(view.character_id))}>
                {t("chat.panel.removeDefault", { name })}
              </button>
            </>
          )}
          <p className="cv-status" role="status">
            {busy || intake.busy ? t("chat.panel.saving") : note}
          </p>
          {(error || intake.error) && (
            <p className="bad" role="alert">
              {error || intake.error}
            </p>
          )}
        </section>

        <section className="cv-sec" aria-label={t("chat.panel.figure")}>
          <h3>{t("chat.panel.figure")}</h3>
          <label className="switch-row">
            <span className="sw-text">
              <b>{t("chat.panel.stage")}</b>
              <span>{t("chat.panel.stage.note")}</span>
            </span>
            <input type="checkbox" role="switch" className="switch" checked={stage} onChange={(e) => onStage(e.target.checked)} />
          </label>
        </section>

        <section className="cv-sec cv-sec-look" aria-label={t("chat.panel.lookSec")}>
          <p className="help">
            {rich("chat.panel.onlyHere", {
              link: (
                <a className="link" href="#/settings/chat">
                  {t("chat.panel.onlyHere.link")}
                </a>
              ),
            })}
          </p>
          <LookFields value={view.look} base={view.global} scope="chat" onChange={setLook} />
        </section>
      </div>
    </aside>
  );
}
