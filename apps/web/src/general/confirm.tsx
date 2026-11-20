// "Erst nachfragen": the question before something is deleted or a chat is rewound, for those who
// switched it on in the settings (tab "Allgemein"). Without it the action runs at once and the
// "Rückgängig" in the toast is the safety net, as before.

import { t } from "../i18n";
import { useEffect, useRef, useState } from "react";
import { Icon, type IconName } from "../ui";
import { general } from "./store";

type Question = {
  title: string;
  text: string;
  /** The button that goes ahead ("Löschen"). */
  action: string;
  icon: IconName;
};

type Asking = Question & { answer: (yes: boolean) => void };

let show: ((q: Asking | null) => void) | null = null;

/** Asks and resolves to the answer. Without a host on screen nothing can be asked: goes ahead. */
function ask(q: Question): Promise<boolean> {
  const open = show;
  if (!open) return Promise.resolve(true);
  return new Promise((resolve) => open({ ...q, answer: resolve }));
}

/** Before a delete of `what` ("„Mara Voss“", "3 Karten"): true = go ahead. */
export function confirmDelete(what: string): Promise<boolean> {
  if (!general().confirm_delete) return Promise.resolve(true);
  return ask({ title: t("shell.confirm.delete.title", { what }), text: t("shell.confirm.undoable"), action: t("shell.confirm.delete.action"), icon: "trash" });
}

/** Before a chat is rewound to a message: true = go ahead. */
export function confirmRewind(): Promise<boolean> {
  if (!general().confirm_rewind) return Promise.resolve(true);
  return ask({
    title: t("shell.confirm.rewind.title"),
    text: t("shell.confirm.rewind.text"),
    action: t("shell.confirm.rewind.action"),
    icon: "rewind",
  });
}

/** The one dialog these questions use; mounted once by the app. */
export function ConfirmHost() {
  const [asking, setAsking] = useState<Asking | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    show = (q) =>
      setAsking((old) => {
        old?.answer(false); // a second question replaces the first
        return q;
      });
    return () => void (show = null);
  }, []);

  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    if (asking && !d.open) d.showModal();
    if (!asking && d.open) d.close();
  }, [asking]);

  const answer = (yes: boolean) => {
    asking?.answer(yes);
    setAsking(null);
  };

  return (
    <dialog
      ref={dialog}
      className="bk-dialog"
      aria-labelledby="gen-ask-title"
      onClose={() => asking && answer(false)}
      onMouseDown={(e) => e.target === e.currentTarget && answer(false)}
    >
      {asking && (
        <div className="bk-dialog-body">
          <h2 id="gen-ask-title">{asking.title}</h2>
          <p>{asking.text}</p>
          <div className="btn-row">
            <button className="btn danger" onClick={() => answer(true)}>
              <Icon name={asking.icon} small />
              {asking.action}
            </button>
            <button className="btn ghost" onClick={() => answer(false)} autoFocus>
              {t("shell.confirm.cancel")}
            </button>
          </div>
          <p className="help">{t("shell.confirm.hint")}</p>
        </div>
      )}
    </dialog>
  );
}
