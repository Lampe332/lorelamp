// One chat's persona, in the chat's bar.

import { useEffect } from "react";
import type { Persona } from "../api";
import { t } from "../i18n";
import { Icon, Portrait } from "../ui";
import { about, CHAT_PERSONA_EVENT } from "./common";
import { MANAGE, PersonaMenu, useMenu } from "./menu";

type ChatPersonaProps = {
  personas: Persona[];
  /** The active persona (null until the library is there). */
  activeId: number | null;
  /** The persona this chat is bound to; null = it follows the active one. */
  bound: number | null;
  onBind: (id: number | null) => void;
};

export function ChatPersona({ personas, activeId, bound, onBind }: ChatPersonaProps) {
  const { open, setOpen, button, close } = useMenu();

  // The quick actions open this menu ("Persona für diesen Chat").
  useEffect(() => {
    const show = () => setOpen(true);
    window.addEventListener(CHAT_PERSONA_EVENT, show);
    return () => window.removeEventListener(CHAT_PERSONA_EVENT, show);
  }, [setOpen]);

  const active = personas.find((p) => p.id === activeId);
  const shown = personas.find((p) => p.id === (bound ?? activeId)) ?? active;
  if (!shown) return null;
  const isBound = bound !== null;
  return (
    <>
      <button
        ref={button}
        type="button"
        className={`chat-persona${isBound ? " bound" : ""}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={isBound ? t("cards.persona.chat.bound", { name: shown.name }) : t("cards.persona.chat.free", { name: shown.name })}
        title={isBound ? t("cards.persona.chat.bound.title") : t("cards.persona.chat.free.title")}
        onClick={() => setOpen((o) => !o)}
        data-log-label="Chat-Persona-Menü"
      >
        <span className="cp-av">
          <Portrait who={shown} kind="persona" className="av" />
          {isBound && (
            <span className="cp-pin" aria-hidden="true">
              <Icon name="pin" small />
            </span>
          )}
        </span>
        <span className="cp-text">
          <small>{isBound ? t("cards.persona.chat.bound.as") : t("cards.persona.chat.free.as")}</small>
          <b>{shown.name}</b>
        </span>
        <Icon name="chevron" small />
      </button>
      {open && (
        <PersonaMenu
          label={t("cards.persona.chat.menu")}
          head={t("cards.persona.chat.menu")}
          note={isBound ? t("cards.persona.chat.bound.note") : t("cards.persona.chat.free.note")}
          options={[
            {
              key: "follow",
              icon: "follow",
              label: t("cards.persona.chat.follow"),
              sub: active ? t("cards.persona.follows.now", { name: active.name }) : t("cards.persona.follows"),
              checked: !isBound,
              onPick: () => isBound && onBind(null),
            },
            ...personas.map((p) => ({
              key: String(p.id),
              persona: p,
              label: p.name,
              sub: about(p),
              checked: bound === p.id,
              onPick: () => bound !== p.id && onBind(p.id),
            })),
          ]}
          links={[MANAGE]}
          anchor={button.current}
          drop="down"
          onClose={close}
        />
      )}
    </>
  );
}
