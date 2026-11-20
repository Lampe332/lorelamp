// The rows of the settings tab "Allgemein" that concern how the app behaves: its start, sending,
// asking first, and its data. Each choice is saved the moment it is made and holds on every device;
// starting with Windows and the data folder belong to the PC.

import { dateFormat, t } from "../i18n";
import { useEffect, useId, useState, type ReactNode } from "react";
import { useFormFactor } from "../app/formfactor";
import { PcOnly } from "../phone/Phone";
import { onOtherDevice } from "../phone/shell";
import { respot } from "../spot";
import { restartTour } from "../tutorial/store";
import { Icon } from "../ui";
import "./general.css";
import { openDataFolder, saveAutostart, saveGeneral, UNDO_CHOICES, useGeneral, type BackupRhythm, type General, type GeneralEdit, type SendKey, type StartPage } from "./store";

// Tables with texts are made when they are drawn, so they are in the language on screen.
const startPages = (): [StartPage, string][] => [
  ["home", t("shell.start.page.home")],
  ["last_chat", t("shell.start.page.last_chat")],
  ["characters", t("shell.start.page.characters")],
];

const rhythms = (): [BackupRhythm, string][] => [
  ["daily", t("shell.data.rhythm.daily")],
  ["weekly", t("shell.data.rhythm.weekly")],
  ["off", t("shell.data.rhythm.off")],
];

const sendKeys = (): { id: SendKey; keys: string[]; note: string }[] => [
  { id: "enter", keys: ["Enter"], note: t("shell.write.enter.note") },
  { id: "ctrl_enter", keys: [t("shell.key.ctrl"), "Enter"], note: t("shell.write.ctrl.note") },
];

/** When the app backs up by itself next, in words. */
export function nextBackupText(g: Pick<General, "backup_rhythm" | "backup_next_ms">, now = Date.now()): string {
  if (g.backup_rhythm === "off" || g.backup_next_ms === null) {
    return t("shell.backup.off");
  }
  if (g.backup_next_ms <= now + 15 * 60_000) return t("shell.backup.soon");
  const at = new Date(g.backup_next_ms);
  const days = Math.round((new Date(g.backup_next_ms).setHours(0, 0, 0, 0) - new Date(now).setHours(0, 0, 0, 0)) / 86_400_000);
  const time = dateFormat({ hour: "2-digit", minute: "2-digit" }).format(at);
  const day = days === 0 ? t("shell.day.today") : days === 1 ? t("shell.day.tomorrow") : t("shell.day.on", { date: dateFormat({ weekday: "long", day: "numeric", month: "long" }).format(at) });
  return t("shell.backup.next", { day, time });
}

/** A choice from a short list: the words on the left, the list at the right edge. */
function PickRow<T extends string | number>({ spot, label, note, value, options, disabled, onPick }: {
  spot: string;
  label: string;
  note: ReactNode;
  value: T;
  options: readonly (readonly [T, string])[];
  disabled: boolean;
  onPick: (value: T) => void;
}) {
  const id = useId();
  return (
    <div className="gen-line" data-spot={spot}>
      <label htmlFor={id}>
        <b>{label}</b>
        <span>{note}</span>
      </label>
      <select
        id={id}
        className="input"
        value={value}
        disabled={disabled}
        data-spot-focus
        onChange={(e) => {
          const picked = options.find(([v]) => String(v) === e.target.value);
          if (picked) onPick(picked[0]);
        }}
      >
        {options.map(([v, name]) => (
          <option key={v} value={v}>
            {name}
          </option>
        ))}
      </select>
    </div>
  );
}

/** "Senden mit": the key that sends, as two cards. */
function SendKeys({ value, off, onPick }: { value: SendKey; off: boolean; onPick: (key: SendKey) => void }) {
  return (
    <>
      <span className="field-label" id="gen-send-label">
        {t("shell.write.sendWith")}
      </span>
      <div className="choices gen-send" role="radiogroup" aria-labelledby="gen-send-label" data-spot="send-key">
        {sendKeys().map((k) => (
          <label className={`choice${off ? " off" : ""}`} key={k.id}>
            <input type="radio" name="gen-send-key" checked={value === k.id} disabled={off} onChange={() => onPick(k.id)} data-log-label="Senden mit" />
            <span className="choice-mark" aria-hidden="true" />
            <span className="choice-text">
              <b>
                {k.keys.map((key, i) => (
                  <span key={key}>
                    {i > 0 && " + "}
                    <kbd>{key}</kbd>
                  </span>
                ))}
              </b>
              <span>{k.note}</span>
            </span>
          </label>
        ))}
      </div>
    </>
  );
}

function Toggle({ spot, label, note, checked, disabled = false, onChange }: { spot: string; label: string; note: string; checked: boolean; disabled?: boolean; onChange: (on: boolean) => void }) {
  return (
    <label className={`switch-row${disabled ? " off" : ""}`} data-spot={spot}>
      <span className="sw-text">
        <b>{label}</b>
        <span>{note}</span>
      </span>
      <input type="checkbox" role="switch" className="switch" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}

/** The welcome tour of the first start, once more from its beginning (src/tutorial/). */
function TourRow() {
  const [problem, setProblem] = useState("");
  return (
    <div className="gen-line gen-tour" data-spot="tutorial">
      <div className="gen-folder-text">
        <b>{t("tutorial.settings.label")}</b>
        <span>{t("tutorial.settings.note")}</span>
        {problem && (
          <span className="bad" role="alert">
            {problem}
          </span>
        )}
      </div>
      <button type="button" className="btn" onClick={() => void restartTour().then((bad) => setProblem(bad && t("tutorial.failed", { error: bad })))} data-spot-focus data-log-label="Einführung neu starten">
        <Icon name="refresh" small />
        {t("tutorial.settings.restart")}
      </button>
    </div>
  );
}

/** The rows "Start", "Schreiben" and "Nachfragen"; `children` (the hints row) sits before "Daten". */
export function GeneralRows({ children }: { children?: ReactNode }) {
  const { value: g, loaded, failed } = useGeneral();
  // Key names only at a desk: a phone or tablet sends with its button (see Chat.tsx).
  const keyboard = useFormFactor() === "desktop";
  const [problem, setProblem] = useState("");
  const [opened, setOpened] = useState(false);
  // A place asked for from the quick actions before the settings arrived is shown now.
  useEffect(() => {
    if (loaded) respot();
  }, [loaded]);
  useEffect(() => {
    if (!opened) return;
    const t = window.setTimeout(() => setOpened(false), 4000);
    return () => window.clearTimeout(t);
  }, [opened]);

  const off = !loaded;
  const change = (edit: GeneralEdit) => void saveGeneral(edit).then((bad) => setProblem(bad && t("shell.data.notSaved", { error: bad })));
  const autostart = (on: boolean, minimized: boolean) => void saveAutostart(on, minimized).then((bad) => setProblem(bad && t("shell.data.notSaved", { error: bad })));
  const openFolder = () =>
    void openDataFolder().then((bad) => {
      setProblem(bad && t("shell.data.folderFailed", { error: bad }));
      setOpened(!bad);
    });

  return (
    <>
      {(problem || (failed && !loaded)) && (
        <p className="bad gen-problem" role="alert">
          {problem || t("shell.data.loadFailed")}
        </p>
      )}
      <div className="set-row">
        <div className="set-label">
          <h2>{t("shell.start.title")}</h2>
          <p>{t("shell.start.text")}</p>
        </div>
        <div className="set-panel" aria-busy={off && !failed}>
          <PickRow
            spot="start-page"
            label={t("shell.start.open")}
            note={t("shell.start.open.note")}
            value={g.start_page}
            options={startPages()}
            disabled={off}
            onPick={(start_page) => change({ start_page })}
          />
          {onOtherDevice ? (
            <PcOnly what={t("shell.start.autostart")}>{null}</PcOnly>
          ) : g.autostart ? (
            <>
              <Toggle
                spot="autostart"
                label={t("shell.start.autostart")}
                note={t("shell.start.autostart.note")}
                checked={g.autostart.on}
                onChange={(on) => autostart(on, g.autostart?.minimized ?? false)}
              />
              <Toggle
                spot="start-minimized"
                label={t("shell.start.minimized")}
                note={t("shell.start.minimized.note")}
                checked={g.autostart.minimized}
                disabled={!g.autostart.on}
                onChange={(minimized) => autostart(true, minimized)}
              />
            </>
          ) : (
            loaded && (
              <div className="gen-note" role="note" data-spot="autostart">
                <b>{t("shell.start.autostart")}</b>
                <span>{t("shell.start.browser")}</span>
              </div>
            )
          )}
          <TourRow />
        </div>
      </div>

      <div className="set-row">
        <div className="set-label">
          <h2>{t("shell.write.title")}</h2>
          <p>{keyboard ? t("shell.write.text") : t("shell.write.textTouch")}</p>
        </div>
        <div className="set-panel">
          {keyboard ? (
            <SendKeys value={g.send_key} off={off} onPick={(send_key) => change({ send_key })} />
          ) : (
            // A finger gets no key names: here the button beside the field sends, the key is the PC's choice.
            <div className="gen-note" role="note" data-spot="send-key">
              <b>{t("shell.write.button")}</b>
              <span>{t("shell.write.button.note")}</span>
            </div>
          )}
        </div>
      </div>

      <div className="set-row">
        <div className="set-label">
          <h2>{t("shell.ask.title")}</h2>
          <p>{t("shell.ask.text")}</p>
        </div>
        <div className="set-panel">
          <Toggle
            spot="confirm-delete"
            label={t("shell.ask.delete")}
            note={t("shell.ask.delete.note")}
            checked={g.confirm_delete}
            disabled={off}
            onChange={(confirm_delete) => change({ confirm_delete })}
          />
          <Toggle
            spot="confirm-rewind"
            label={t("shell.ask.rewind")}
            note={t("shell.ask.rewind.note")}
            checked={g.confirm_rewind}
            disabled={off}
            onChange={(confirm_rewind) => change({ confirm_rewind })}
          />
          <PickRow
            spot="undo-time"
            label={t("shell.ask.undo")}
            note={t("shell.ask.undo.note")}
            value={g.undo_secs}
            options={UNDO_CHOICES.map((s) => [s, t("shell.ask.seconds", { n: s })] as const)}
            disabled={off}
            onPick={(undo_secs) => change({ undo_secs })}
          />
        </div>
      </div>

      {children}

      <div className="set-row">
        <div className="set-label">
          <h2>{t("shell.data.title")}</h2>
          <p>{t("shell.data.text")}</p>
        </div>
        <div className="set-panel">
          <PickRow
            spot="backup-rhythm"
            label={t("shell.data.auto")}
            note={
              <>
                <span role="status">{loaded ? nextBackupText(g) : "…"}</span>{" "}
                <a href="#/settings/backups">
                  {t("shell.data.see")}
                </a>
              </>
            }
            value={g.backup_rhythm}
            options={rhythms()}
            disabled={off}
            onPick={(backup_rhythm) => change({ backup_rhythm })}
          />
          {onOtherDevice ? (
            <PcOnly what={t("shell.data.folder")}>{null}</PcOnly>
          ) : (
            <div className="gen-line gen-folder" data-spot="data-folder">
              <div className="gen-folder-text">
                <b>{t("shell.data.folder")}</b>
                <code className="gen-path">{g.data_dir ?? "…"}</code>
              </div>
              <button type="button" className="btn" disabled={off || !g.data_dir} onClick={openFolder} data-spot-focus>
                <Icon name="folder" small />
                {t("shell.data.open")}
              </button>
              {opened && (
                <p className="help gen-opened" role="status">
                  {t("shell.data.opened")}
                </p>
              )}
            </div>
          )}
        </div>
      </div>
    </>
  );
}
