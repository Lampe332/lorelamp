// "Du spielst als …" at the foot of the sidebar (and in the phone's top bar).

import type { Persona } from "../api";
import { t } from "../i18n";
import { Icon, Portrait } from "../ui";
import { about } from "./common";
import { MANAGE, PersonaMenu, useMenu } from "./menu";

type SwitchProps = {
  personas: Persona[];
  activeId: number;
  onActivate: (id: number) => void;
  /** Where the button sits; on a phone the menu becomes a sheet from the bottom either way. */
  place: "side" | "top";
};

export function PersonaSwitch({ personas, activeId, onActivate, place }: SwitchProps) {
  const { open, setOpen, button, close } = useMenu();
  const active = personas.find((p) => p.id === activeId) ?? personas[0];
  if (!active) return null;
  return (
    <>
      <button
        ref={button}
        type="button"
        className={`persona-btn at-${place}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        data-log-label="Persona-Menü"
        title={t("shell.persona.switch")}
      >
        <Portrait who={active} kind="persona" className="av" />
        <span className="pb-text">
          <span className="faint">{t("shell.persona.playAs")}</span>
          <b>{active.name}</b>
        </span>
        <Icon name="updown" small />
      </button>
      {open && (
        <PersonaMenu
          label={t("shell.persona.switch")}
          head={t("shell.persona.playAs")}
          options={personas.map((p) => ({
            key: String(p.id),
            persona: p,
            label: p.name,
            sub: about(p),
            checked: p.id === active.id,
            onPick: () => p.id !== active.id && onActivate(p.id),
          }))}
          links={[{ href: "#/personas/new", label: t("shell.persona.new"), icon: "plus" }, MANAGE]}
          anchor={button.current}
          drop="up"
          onClose={close}
        />
      )}
    </>
  );
}
