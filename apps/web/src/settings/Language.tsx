// Settings → Allgemein, first row: the app's language. Each language stands there under its own
// name, so someone who cannot read the language on screen still finds theirs. The server keeps the
// choice (`PUT /settings {language}`): every connected device follows, and the figures answer in
// that language from the next reply on.

import { useState } from "react";
import { errorText, postJson } from "../api";
import { lang, LANGS, setLanguage, t, type LangId } from "../i18n";
import { logEvent } from "../log";
import { openSetting } from "../spot";

export function LanguageRow() {
  const [saving, setSaving] = useState<LangId | null>(null);
  const [problem, setProblem] = useState("");
  const current = saving ?? lang();

  const pick = async (id: LangId) => {
    if (id === lang() || saving) return;
    logEvent("info", "settings", "Sprache geändert", { data: { language: id } });
    setSaving(id);
    setProblem("");
    try {
      await postJson("/settings", { language: id }, "PUT");
      // The app is drawn anew in the new language; this row stays in view and keeps the focus.
      openSetting("#/settings", "language");
      setLanguage(id);
    } catch (e) {
      setProblem(t("shell.toast.languageNotSaved", { error: errorText(e) }));
      setSaving(null);
    }
  };

  return (
    <div className="set-row">
      <div className="set-label">
        <h2 id="gen-language-label">{t("shell.general.language.title")}</h2>
        <p>{t("shell.general.language.text")}</p>
      </div>
      <div className="set-panel">
        <div className="choices gen-languages" role="radiogroup" aria-labelledby="gen-language-label" data-spot="language">
          {LANGS.map((l) => (
            <label className={`choice${saving ? " off" : ""}`} key={l.id}>
              <input type="radio" name="gen-language" checked={current === l.id} disabled={saving !== null} onChange={() => void pick(l.id)} data-log-label="Sprache" />
              <span className="choice-mark" aria-hidden="true" />
              <span className="choice-text">
                <b lang={l.id}>{l.name}</b>
              </span>
            </label>
          ))}
        </div>
        <p className="help">{t("shell.general.language.note")}</p>
        {problem && (
          <p className="bad" role="alert">
            {problem}
          </p>
        )}
      </div>
    </div>
  );
}
