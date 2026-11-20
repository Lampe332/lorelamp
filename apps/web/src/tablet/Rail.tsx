// A tablet's navigation: a narrow rail at the left edge in place of the desktop's sidebar. Every
// destination is an icon with its name under it (at least 48 px to touch); the last chats are their
// figures' pictures, the model's state is one dot, and who you play as is your picture at the foot.

import { t } from "../i18n";
import { useEffect, type ComponentProps } from "react";
import type { Library, Persona } from "../api";
import type { Here } from "../app/Sidebar";
import { StatusPill } from "../app/Sidebar";
import { who } from "../hub/blocks";
import { openDownloads, useDownloads } from "../models/store";
import { PersonaSwitch } from "../persona/PersonaSwitch";
import { BrandMark, Icon, Portrait, hueStyle, type IconName } from "../ui";
import { holdFigures } from "./data";
import "./tablet.css";

type RailProps = {
  library: Library | null;
  here: Here;
  status: ComponentProps<typeof StatusPill>;
  onSearch: () => void;
  personas: Persona[];
  activePersona: number | null;
  onActivate: (id: number) => void;
};

function Item({ href, icon, label, current, badge, log }: { href: string; icon: IconName; label: string; current: boolean; badge?: number; log?: string }) {
  return (
    <a className="tb-item" href={href} aria-current={current ? "page" : undefined} data-log-label={log}>
      <span className="tb-item-ic">
        <Icon name={icon} />
        {badge ? <span className="tb-item-badge num">{badge}</span> : null}
      </span>
      <span className="tb-item-name">{label}</span>
    </a>
  );
}

export function Rail({ library, here, status, onSearch, personas, activePersona, onActivate }: RailProps) {
  const downloads = openDownloads(useDownloads().list).length;
  // One entry per character: its most recent chat.
  const recent = (library?.recent ?? []).filter((r, i, all) => all.findIndex((x) => x.chat.character_id === r.chat.character_id) === i).slice(0, 3);
  // A chat opened from here (or the start screen) shows its figure at once.
  useEffect(() => {
    for (const r of library?.recent ?? []) {
      const character = library?.characters.find((c) => c.id === r.chat.character_id);
      if (character) holdFigures(character, [r.chat]);
    }
  }, [library]);
  return (
    <aside className="tb-rail" aria-label={t("shell.nav.navigation")}>
      <a className="tb-brand" href="#/" aria-label={t("shell.nav.brandHome")}>
        <BrandMark className="tb-brand-mark" />
      </a>
      <button type="button" className="tb-item tb-search" onClick={onSearch} aria-label={t("shell.nav.searchLabel")}>
        <span className="tb-item-ic">
          <Icon name="search" />
        </span>
        <span className="tb-item-name">{t("shell.nav.search")}</span>
      </button>
      <nav className="tb-nav" aria-label={t("shell.nav.areas")}>
        <Item href="#/" icon="home" label={t("shell.nav.start")} current={here.hub} />
        <Item href="#/chars" icon="users" label={t("shell.nav.chars")} current={here.library} />
        <Item href="#/personas" icon="persona" label={t("shell.nav.personas")} current={here.personas} />
        <Item href="#/settings/suggestions" icon="model" label={t("shell.nav.models")} current={here.models ?? false} badge={downloads} log="Modelle finden" />
      </nav>
      {recent.length > 0 && (
        <nav className="tb-recent" aria-label={t("shell.nav.recentChats")}>
          {recent.map((r) => (
            <a
              key={r.chat.id}
              className="tb-recent-link"
              href={`#/chat/${r.chat.id}`}
              aria-current={r.chat.id === here.chatId ? "page" : undefined}
              aria-label={t("shell.nav.chatWith", { name: r.name })}
              style={hueStyle(r.name)}
              data-log-label="Letzter Chat"
            >
              <Portrait who={who(r)} className="av" />
            </a>
          ))}
        </nav>
      )}
      <div className="tb-foot">
        <a className="tb-item tb-status" href="#/settings/model" data-log-label="Modell-Status">
          <span className="tb-item-ic">
            <StatusPill {...status} />
          </span>
          <span className="tb-item-name" aria-hidden="true">
            {t("shell.nav.model")}
          </span>
        </a>
        <nav className="tb-nav" aria-label="App">
          <Item href="#/settings" icon="gear" label={t("shell.nav.settings")} current={here.settings} />
        </nav>
        {library && activePersona !== null && <PersonaSwitch personas={personas} activeId={activePersona} onActivate={onActivate} place="side" />}
      </div>
    </aside>
  );
}
