// The app's navigation: the sidebar on a desktop, the top bar and the bottom tab bar on a phone, and
// the model-state pill both of them show.

import { t } from "../i18n";
import { modelFacts, modelName, num, when, type EngineInfo, type EngineStatus, type Library, type Persona, type Timings } from "../api";
import { cachePct, who } from "../hub/blocks";
import { DownloadsBadge, ModelsNavLink } from "../models/Downloads";
import { PersonaSwitch } from "../persona/PersonaSwitch";
import { MoreTab } from "../phone/More";
import { inApp, onOtherDevice, RESCAN_URL } from "../phone/shell";
import { BrandMark, Icon, Portrait } from "../ui";
import { problemLabel, problemLine, problemOf } from "./engineProblem";

/** Where the user is, for the links that mark the current page. */
export type Here = { hub: boolean; library: boolean; settings: boolean; personas: boolean; chatId: number | null; models?: boolean };

type PillProps = {
  online: boolean;
  engine: EngineStatus;
  engineNote: string;
  label: string;
  /** The newest answer's measurements (live, else the server's last ones). */
  timings: Timings | null;
  /** What `/engine` says about the model; null until the first answer. */
  info: EngineInfo | null;
  /** A profile switch is under way (the model server restarts). */
  switching: boolean;
  /** The profile it goes to, if known. */
  switchTo: number | null;
  /** Unloaded to free the graphics memory; the next message loads it again. */
  unloaded?: boolean;
};

/** The model's state. Once it is ready: its name, quant and context, and the newest answer's speed
 *  and cache share. While a profile switch runs: which profile comes. */
export function StatusPill({ online, engine, engineNote, label, timings, info, switching, switchTo, unloaded = false }: PillProps) {
  const pct = timings && cachePct(timings);
  const name = modelName(info?.model);
  const idle = online && unloaded && !switching;
  const ready = online && engine === "ready" && !switching && !idle;
  // Another program holds the port, or the model stopped after crashes: say that, not a hopeful "startet".
  const problem = online && !switching && !idle ? problemOf(info) : null;
  const facts = modelFacts(info?.model);
  const file = info?.model.file;
  const title = idle
    ? t("shell.pill.unloadedTitle")
    : [name && file && !file.startsWith(name) ? `${name}\n${file}` : name, engineNote].filter(Boolean).join("\n");
  const dot = !online ? "offline" : idle ? "idle" : problem ? "offline" : switching || (engine === "offline" && !engineNote) ? "loading" : engine;
  return (
    <span className="status-pill" role="status" title={title || undefined}>
      <i className={`dot ${dot}`} />
      {ready && name ? (
        <b className="sp-name">
          <span className="sr">{label}: </span>
          {name}
        </b>
      ) : (
        <b>{problem ? problemLabel(problem) : label}</b>
      )}
      {idle ? (
        <span className="hide-sm sp-line sp-wrap">{t("shell.pill.loadsOnSend")}</span>
      ) : problem ? (
        <span className="hide-sm sp-line sp-wrap r2-pill-problem">{problemLine(problem)}</span>
      ) : online && switching ? (
        // The profile it goes to, once /engine names it (until then the old one would show).
        info?.profile && info.profile.id === switchTo && <span className="hide-sm sp-line">{t("shell.pill.to", { name: info.profile.name })}</span>
      ) : online && info?.bench_profile_id != null ? (
        <span className="hide-sm sp-line">{t("shell.pill.bench")}</span>
      ) : (
        online &&
        facts && (
          <span className="hide-sm num sp-line">
            {facts}
            {info?.profile && (
              <>
                {" "}
                <span className="sep">·</span>{" "}
                <span className={info.profile.measured ? "sp-measured" : undefined} title={info.profile.measured ? t("shell.pill.measuredTitle") : t("shell.pill.estimatedTitle")}>
                  {info.profile.measured ? t("shell.pill.measured") : t("shell.pill.estimated")}
                </span>
              </>
            )}
          </span>
        )
      )}
      {ready && timings && (
        <span className="hide-sm num">
          {t("shell.pill.tokens", { n: Math.round(timings.tok_s) })}
          {pct !== null && (
            <>
              {" "}
              <span className="sep">·</span> {t("shell.pill.cache")} <span className="data">{num(pct)} %</span>
            </>
          )}
        </span>
      )}
    </span>
  );
}

/** The sidebar's button on the pill: unload the model (frees the graphics memory) or load it. */
export type ModelLoad = { unloaded: boolean; onToggle: () => void } | null;

function ModelLoadButton({ unloaded, onToggle }: NonNullable<ModelLoad>) {
  return (
    <button
      type="button"
      className="status-load"
      onClick={onToggle}
      title={unloaded ? t("shell.load.loadTitle") : t("shell.load.unloadTitle")}
      aria-label={unloaded ? t("shell.load.loadAria") : t("shell.load.unloadAria")}
    >
      {unloaded ? t("shell.load.load") : t("shell.load.unload")}
    </button>
  );
}

/** The pill as a way into the settings tab "Modell". */
function StatusLink(props: PillProps) {
  return (
    <a className="status-link" href="#/settings/model" data-log-label="Modell-Status">
      <StatusPill {...props} />
    </a>
  );
}

function MainLinks({ here }: { here: Here }) {
  return (
    <>
      <a href="#/" aria-current={here.hub ? "page" : undefined}>
        <Icon name="home" />
        {t("shell.nav.start")}
      </a>
      <a href="#/chars" aria-current={here.library ? "page" : undefined}>
        <Icon name="users" />
        {t("shell.nav.chars")}
      </a>
    </>
  );
}

function SettingsLink({ here }: { here: Here }) {
  return (
    <a href="#/settings" aria-current={here.settings ? "page" : undefined}>
      <Icon name="gear" />
      {t("shell.nav.settings")}
    </a>
  );
}

/** "Handy verbinden" on the PC; on another device the connection, in the Android app straight to another PC. */
function PhoneLink() {
  return inApp ? (
    <a href={RESCAN_URL} data-log-label="Anderen PC verbinden">
      <Icon name="qr" />
      {t("shell.nav.otherPc")}
    </a>
  ) : (
    <a href="#/settings/phone">
      <Icon name="qr" />
      {onOtherDevice ? t("shell.nav.connection") : t("shell.nav.phone")}
    </a>
  );
}

type PersonaProps = { personas: Persona[]; activePersona: number | null; onActivate: (id: number) => void };

type SidebarProps = PersonaProps & {
  library: Library | null;
  here: Here;
  status: PillProps;
  onSearch: () => void;
  /** The unload/load button on the model status (only when this app runs the model). */
  modelLoad?: ModelLoad;
};

export function Sidebar({ library, here, status, onSearch, modelLoad = null, personas, activePersona, onActivate }: SidebarProps) {
  // One entry per character: its most recent chat.
  const recent = (library?.recent ?? [])
    .filter((r, i, all) => all.findIndex((x) => x.chat.character_id === r.chat.character_id) === i)
    .slice(0, 4);
  return (
    <aside className="side side-tidy">
      <a className="brand" href="#/">
        <BrandMark className="brand-mark" />
        <b>Lorelamp</b>
      </a>
      <button type="button" className="side-search" onClick={onSearch} aria-label={t("shell.nav.searchLabelKey")} aria-keyshortcuts="Control+K">
        <Icon name="search" small />
        <span>{t("shell.nav.search")}</span>
        <kbd>{t("shell.key.ctrlK")}</kbd>
      </button>
      <div className="side-group">
        <h2>{t("shell.nav.play")}</h2>
        <nav className="nav" aria-label={t("shell.nav.play")}>
          <MainLinks here={here} />
        </nav>
      </div>
      {recent.length > 0 && (
        <div className="side-group">
          <h2>{t("shell.nav.recent")}</h2>
          <nav className="recent" aria-label={t("shell.nav.recentChats")}>
            {recent.map((r) => (
              <a key={r.chat.id} href={`#/chat/${r.chat.id}`} aria-current={r.chat.id === here.chatId ? "page" : undefined} data-log-label="Letzter Chat">
                <Portrait who={who(r)} className="av" />
                <span className="rc">
                  <span className="rc-name">{r.name}</span>
                  <span className="rc-sub">{r.chat.title || when(r.chat.updated_at)}</span>
                </span>
              </a>
            ))}
          </nav>
        </div>
      )}
      <div className="side-group">
        <h2>{t("shell.nav.world")}</h2>
        <nav className="nav" aria-label={t("shell.nav.world")}>
          <a href="#/personas" aria-current={here.personas ? "page" : undefined}>
            <Icon name="persona" />
            {t("shell.nav.personas")}
          </a>
        </nav>
      </div>
      <div className="side-group">
        <h2>{t("shell.nav.system")}</h2>
        <nav className="nav" aria-label={t("shell.nav.system")}>
          <ModelsNavLink current={here.models ?? false} />
          {/* Planned places are not in the navigation; the start page names them once at its foot. */}
          <PhoneLink />
        </nav>
      </div>
      <div className="side-foot">
        <div className={`status-row${modelLoad ? " with-load" : ""}`}>
          <StatusLink {...status} />
          {modelLoad && <ModelLoadButton {...modelLoad} />}
        </div>
        <nav className="nav" aria-label="App">
          <SettingsLink here={here} />
        </nav>
        {library && activePersona !== null && <PersonaSwitch personas={personas} activeId={activePersona} onActivate={onActivate} place="side" />}
      </div>
    </aside>
  );
}

/** A phone's bar at the top: home, the model's state, search and who the user plays as. */
export function TopBar({ library, status, onSearch, personas, activePersona, onActivate }: Omit<SidebarProps, "here" | "modelLoad">) {
  return (
    <header className="top">
      <a href="#/" className="top-home" aria-label={t("shell.nav.home")}>
        <BrandMark className="top-mark" />
      </a>
      <StatusLink {...status} />
      <DownloadsBadge />
      <button type="button" className="icon-btn top-search" onClick={onSearch} aria-label={t("shell.nav.searchLabel")}>
        <Icon name="search" />
      </button>
      {library && activePersona !== null && <PersonaSwitch personas={personas} activeId={activePersona} onActivate={onActivate} place="top" />}
    </header>
  );
}

/** A phone's bar at the bottom (on the top-level screens only); "Mehr" holds the rest of the sidebar. */
export function TabBar({ here, status, onSearch }: { here: Here; status: PillProps; onSearch: () => void }) {
  return (
    <nav className="tabbar" aria-label={t("shell.nav.areas")}>
      <MainLinks here={here} />
      <SettingsLink here={here} />
      <MoreTab current={here.personas || !!here.models} onSearch={onSearch} status={<StatusPill {...status} />} />
    </nav>
  );
}
