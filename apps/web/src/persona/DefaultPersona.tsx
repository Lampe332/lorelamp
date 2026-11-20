// The persona new chats with a character start with (character settings).

import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { errorText, postJson, type Persona } from "../api";
import { t } from "../i18n";
import { logEvent } from "../log";
import { Icon, Portrait } from "../ui";
import { about } from "./common";

type DefaultProps = {
  characterId: number;
  characterName: string;
  personas: Persona[];
  /** The saved choice (from the library, so other devices' changes show). */
  value: number | null;
};

/** Saved at once: it changes nothing until the next new chat. */
export function DefaultPersona({ characterId, characterName, personas, value }: DefaultProps) {
  const [pending, setPending] = useState<{ id: number | null } | null>(null);
  const [state, setState] = useState<"idle" | "saving" | "saved">("idle");
  const [error, setError] = useState("");
  const group = useRef<HTMLDivElement>(null);
  const shown = pending ? pending.id : value;

  // The library has caught up with the choice.
  useEffect(() => {
    if (pending && pending.id === value) setPending(null);
  }, [pending, value]);

  const choose = async (id: number | null) => {
    if (id === shown) return;
    logEvent("info", "settings", "Standard-Persona gesetzt", { character_id: characterId, data: { persona_id: id } });
    setPending({ id });
    setState("saving");
    setError("");
    try {
      await postJson(`/characters/${characterId}/default-persona`, { persona_id: id }, "PUT");
      setState("saved");
    } catch (e) {
      setPending(null);
      setError(errorText(e));
      setState("idle");
    }
  };

  // Arrow keys move along the choices, like any group of radio buttons.
  const onKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const all = [...(group.current?.querySelectorAll<HTMLElement>('[role="radio"]') ?? [])];
    const at = all.indexOf(document.activeElement as HTMLElement);
    const step = e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : e.key === "ArrowUp" || e.key === "ArrowLeft" ? -1 : 0;
    if (!step || at < 0) return;
    e.preventDefault();
    all[(at + step + all.length) % all.length]?.focus();
  };

  const options: { id: number | null; persona?: Persona; label: string; sub: string }[] = [
    { id: null, label: t("cards.persona.default.none"), sub: t("cards.persona.follows") },
    ...personas.map((p) => ({ id: p.id, persona: p, label: p.name, sub: about(p) })),
  ];
  return (
    <section className="set-row" data-spot="default-persona">
      <div className="set-label">
        <h2>{t("cards.persona.default")}</h2>
        <p>{t("cards.persona.default.text", { name: characterName })}</p>
      </div>
      <div className="set-panel">
        <div className="dp" role="radiogroup" aria-label={t("cards.persona.default.label")} ref={group} onKeyDown={onKey}>
          {options.map((o) => (
            <button
              key={o.id ?? "none"}
              type="button"
              role="radio"
              aria-checked={o.id === shown}
              tabIndex={o.id === shown ? 0 : -1}
              className="dp-opt"
              onClick={() => void choose(o.id)}
              data-log-label="Standard-Persona wählen"
            >
              {o.persona ? (
                <Portrait who={o.persona} kind="persona" className="av" />
              ) : (
                <span className="av dp-none" aria-hidden="true">
                  <Icon name="follow" small />
                </span>
              )}
              <span className="dp-text">
                <b>{o.label}</b>
                <small>{o.sub}</small>
              </span>
              <span className="dp-mark" aria-hidden="true">
                {o.id === shown && <Icon name="check" small />}
              </span>
            </button>
          ))}
        </div>
        <p className={error ? "help bad" : "help"} role="status">
          {error
            ? t("cards.save.failed", { error })
            : state === "saving"
              ? t("cards.save.saving")
              : state === "saved"
                ? t("cards.persona.default.saved")
                : t("cards.persona.default.idle")}
        </p>
      </div>
    </section>
  );
}
