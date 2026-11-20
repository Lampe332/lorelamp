// A phone's fourth tab, "Mehr": everything the sidebar holds on a desktop and the three tabs do not
// (personas, models, import, search, the connection) in a sheet that rises from the bottom bar. It
// can be closed by its backdrop, the back key, or by pulling it down.

import { t } from "../i18n";
import { useCallback, useEffect, useRef, useState, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "../ui";
import { inApp, onOtherDevice, RESCAN_URL } from "./shell";
import "./phone.css";

type Row = { icon: IconName; name: string; note: string; href?: string; run?: () => void; log?: string };

/** How far (px) or how fast (px/ms) the sheet has to be pulled down to close. */
const PULL_CLOSE = 90;
const FLICK_CLOSE = 0.6;
const OUT_MS = 200;

type Props = {
  /** One of the sheet's own pages is open (personas, models): the tab shows as the current one. */
  current: boolean;
  onSearch: () => void;
  /** The model's state, shown as the sheet's first row (the way into the tab "Modell"). */
  status: ReactNode;
};

export function MoreTab({ current, onSearch, status }: Props) {
  const [open, setOpen] = useState(false);
  const shut = useCallback(() => setOpen(false), []);
  return (
    <>
      <button type="button" className="ph-tab" aria-haspopup="dialog" aria-expanded={open} data-current={current || open || undefined} onClick={() => setOpen(true)}>
        <Icon name="more" />
        {t("shell.nav.more")}
      </button>
      {open && <MoreSheet onClose={shut} onSearch={onSearch} status={status} />}
    </>
  );
}

function MoreSheet({ onClose, onSearch, status }: { onClose: () => void } & Omit<Props, "current">) {
  const dialog = useRef<HTMLDialogElement>(null);
  const pull = useRef<{ y: number; t: number; dy: number } | null>(null);
  const timer = useRef(0);

  /** What to do once the sheet's own history entry is gone ("Suchen" opens the search window). */
  const after = useRef<(() => void) | null>(null);

  useEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
    // The sheet is a step in the history of its own, so a phone's back key (or gesture) closes
    // the sheet instead of leaving the page under it.
    if (!history.state?.phSheet) history.pushState({ ...history.state, phSheet: true }, "");
    // That step was left (back key), or another page was opened from the sheet: the sheet is done.
    const gone = () => {
      onClose();
      after.current?.();
    };
    window.addEventListener("popstate", gone);
    window.addEventListener("hashchange", gone);
    return () => {
      window.removeEventListener("popstate", gone);
      window.removeEventListener("hashchange", gone);
    };
  }, [onClose]);
  useEffect(() => () => clearTimeout(timer.current), []);

  /** Takes the sheet's step out of the history again, which removes the sheet (see `gone`). */
  const leave = () => (history.state?.phSheet ? history.back() : onClose());

  /** Slides the sheet out, then removes it. */
  const close = () => {
    const d = dialog.current;
    if (!d || d.dataset.closing) return;
    d.dataset.closing = "1";
    timer.current = window.setTimeout(leave, OUT_MS);
  };

  /** A row's page takes the place of the sheet's step, so "back" from it returns to the page the
   *  sheet was opened on, not to an empty step. */
  const go = (e: MouseEvent<HTMLAnchorElement>) => {
    e.preventDefault();
    if (e.currentTarget.href === location.href) close();
    else location.replace(e.currentTarget.href);
  };

  // Pulling the sheet down by its head follows the finger; far or fast enough closes it.
  const onDown = (e: PointerEvent<HTMLElement>) => {
    pull.current = { y: e.clientY, t: e.timeStamp, dy: 0 };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onMove = (e: PointerEvent<HTMLElement>) => {
    const p = pull.current;
    const d = dialog.current;
    if (!p || !d) return;
    p.dy = Math.max(0, e.clientY - p.y);
    d.style.transition = "none";
    d.style.transform = `translateY(${p.dy}px)`;
  };
  const onUp = (e: PointerEvent<HTMLElement>) => {
    const p = pull.current;
    const d = dialog.current;
    pull.current = null;
    if (!p || !d) return;
    const speed = p.dy / Math.max(1, e.timeStamp - p.t);
    d.style.transition = "";
    if (p.dy > PULL_CLOSE || (p.dy > 24 && speed > FLICK_CLOSE)) {
      // Goes on from where the finger left it.
      d.style.setProperty("--ph-from", `${p.dy}px`);
      d.style.transform = "";
      close();
    } else d.style.transform = "";
  };

  const rows: Row[] = [
    { icon: "persona", name: t("shell.nav.personas"), note: t("shell.more.personas.note"), href: "#/personas" },
    { icon: "model", name: t("shell.nav.models"), note: t("shell.more.models.note"), href: "#/settings/suggestions" },
    { icon: "upload", name: t("shell.more.import"), note: t("shell.more.import.note"), href: "#/import" },
    { icon: "search", name: t("shell.nav.search"), note: t("shell.more.search.note"), run: onSearch },
    inApp
      ? { icon: "qr", name: t("shell.nav.otherPc"), note: t("shell.more.otherPc.note"), href: RESCAN_URL, log: "Anderen PC verbinden" }
      : onOtherDevice
        ? { icon: "qr", name: t("shell.nav.connection"), note: t("shell.more.connection.note"), href: "#/settings/phone" }
        : { icon: "qr", name: t("shell.nav.phone"), note: t("shell.more.phone.note"), href: "#/settings/phone" },
  ];

  // Outside the bar (the bar styles its own links), on the page itself.
  return createPortal(
    <dialog
      ref={dialog}
      className="ph-sheet"
      aria-label={t("shell.nav.more")}
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onClick={(e) => e.target === e.currentTarget && close()}
    >
      <div className="ph-sheet-body">
        <div className="ph-sheet-head" onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp}>
          <i className="ph-grab" aria-hidden="true" />
          <h2>{t("shell.nav.more")}</h2>
        </div>
        <a className="ph-model" href="#/settings/model" data-log-label="Modell-Status" onClick={go}>
          {status}
          <Icon name="chevron" small />
        </a>
        <nav className="ph-rows" aria-label={t("shell.nav.more")}>
          {rows.map((r) => {
            const inner = (
              <>
                <span className="ph-row-ic">
                  <Icon name={r.icon} />
                </span>
                <span className="ph-row-text">
                  <b>{r.name}</b>
                  <small>{r.note}</small>
                </span>
              </>
            );
            return r.href ? (
              <a key={r.name} className="ph-row" href={r.href} data-log-label={r.log} onClick={go}>
                {inner}
              </a>
            ) : (
              <button
                key={r.name}
                type="button"
                className="ph-row"
                onClick={() => {
                  if (history.state?.phSheet) {
                    after.current = r.run ?? null;
                    history.back();
                  } else {
                    onClose();
                    r.run?.();
                  }
                }}
              >
                {inner}
              </button>
            );
          })}
        </nav>
      </div>
    </dialog>,
    document.body,
  );
}
