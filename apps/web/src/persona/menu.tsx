// The persona menu shared by the switcher and a chat's persona button.

import { texts } from "../i18n";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { Persona } from "../api";
import { Icon, Portrait, type IconName } from "../ui";

type MenuOption = {
  key: string;
  /** A persona's picture, or an icon for "follow the active one". */
  persona?: Persona;
  icon?: IconName;
  label: string;
  sub: string;
  checked: boolean;
  onPick: () => void;
};

export type MenuLink = { href: string; label: string; icon: IconName };

type MenuProps = {
  label: string;
  head: string;
  /** A short line under the head. */
  note?: string;
  options: MenuOption[];
  links: MenuLink[];
  anchor: HTMLElement | null;
  /** Desktop: opens above the button (sidebar) or below it (a bar at the top). A phone gets a sheet. */
  drop: "up" | "down";
  /** `refocus`: give the focus back to the button (Escape, a pick), not when a click went elsewhere. */
  onClose: (refocus: boolean) => void;
};

/** Room around the menu and its smallest width on a desktop. */
const EDGE = 8;
const MIN_WIDTH = 272;

const phone = () => matchMedia("(max-width: 767px)").matches;

export function PersonaMenu({ label, head, note, options, links, anchor, drop, onClose }: MenuProps) {
  const [sheet] = useState(phone);
  const [place, setPlace] = useState<CSSProperties | undefined>(undefined);
  const menu = useRef<HTMLDivElement>(null);

  // Desktop: beside the button's edge, at least MIN_WIDTH wide, inside the window.
  useLayoutEffect(() => {
    if (sheet || !anchor) return;
    const fit = () => {
      const r = anchor.getBoundingClientRect();
      const width = Math.min(Math.max(r.width, drop === "down" ? MIN_WIDTH + 40 : MIN_WIDTH), window.innerWidth - 2 * EDGE);
      if (drop === "up") {
        const left = Math.min(Math.max(r.left, EDGE), window.innerWidth - EDGE - width);
        setPlace({ left, width, bottom: window.innerHeight - r.top + 6, maxHeight: Math.max(160, r.top - 6 - EDGE) });
      } else {
        const left = Math.min(Math.max(r.right - width, EDGE), window.innerWidth - EDGE - width);
        setPlace({ left, width, top: r.bottom + 6, maxHeight: Math.max(160, window.innerHeight - r.bottom - 6 - EDGE) });
      }
    };
    fit();
    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  }, [anchor, sheet, drop]);

  // The focus starts on the current choice.
  useEffect(() => {
    menu.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
  }, []);

  const items = () => [...(menu.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? [])];
  const onKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const all = items();
    const at = all.indexOf(document.activeElement as HTMLElement);
    const to = (i: number) => {
      e.preventDefault();
      all[(i + all.length) % all.length]?.focus();
    };
    if (e.key === "ArrowDown") to(at + 1);
    else if (e.key === "ArrowUp") to(at < 0 ? all.length - 1 : at - 1);
    else if (e.key === "Home") to(0);
    else if (e.key === "End") to(all.length - 1);
    else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose(true);
    } else if (e.key === "Tab") onClose(false);
  };

  return createPortal(
    <>
      <div className={`pm-scrim${sheet ? " sheet" : ""}`} onPointerDown={() => onClose(false)} aria-hidden="true" />
      <div
        ref={menu}
        className={`pm${sheet ? " sheet" : drop === "down" ? " down" : ""}`}
        role="menu"
        aria-label={label}
        style={place}
        onKeyDown={onKey}
      >
        {sheet && <i className="pm-grab" aria-hidden="true" />}
        <div className="pm-head">
          {head}
          {note && <small>{note}</small>}
        </div>
        <div className="pm-list">
          {options.map((o) => (
            <button
              key={o.key}
              type="button"
              role="menuitemradio"
              aria-checked={o.checked}
              className="pm-item"
              tabIndex={-1}
              onClick={() => {
                onClose(true);
                o.onPick();
              }}
              data-log-label="Persona wählen"
            >
              {o.persona ? (
                <Portrait who={o.persona} kind="persona" className="av" />
              ) : (
                <span className="pm-ico">
                  <Icon name={o.icon ?? "persona"} small />
                </span>
              )}
              <span className="pm-text">
                <b>{o.label}</b>
                <small>{o.sub}</small>
              </span>
              {o.checked && <Icon name="check" small />}
            </button>
          ))}
        </div>
        <div className="pm-acts">
          {links.map((l) => (
            <a key={l.href} role="menuitem" tabIndex={-1} className="pm-item pm-act" href={l.href} onClick={() => onClose(false)}>
              <span className="pm-ico">
                <Icon name={l.icon} small />
              </span>
              {l.label}
            </a>
          ))}
        </div>
      </div>
    </>,
    document.body,
  );
}

/** Open state of a menu button; the menu closes when the page changes underneath it. */
export function useMenu() {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    window.addEventListener("hashchange", close);
    return () => window.removeEventListener("hashchange", close);
  }, [open]);
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };
  return { open, setOpen, button, close };
}

export const MANAGE: MenuLink = texts({ href: "#/personas", icon: "persona" }, { label: "shell.persona.manage" });
