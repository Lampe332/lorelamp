// App settings: "Allgemein" (who the user plays as, the colour theme, the hint when an answer is ready,
// and general/Rows.tsx: the app's start, the key that sends, asking first, backups and the data folder),
// "Chat" (settings/ChatTab.tsx: how every chat reads),
// "Modell" (settings/ModelTab.tsx: the simple view), "Experte" (settings/Expert.tsx: every model
// setting exactly), "Sicherungen" (settings/Backups.tsx: room used, the backups by kind, back up now,
// restore, delete), "Handy" (phone/Phone.tsx: phones in the Wi-Fi, QR code, pairing code; on a phone:
// the connection) and "Logs" (the activity log: view, filter, export, clear).

import { dateFormat, raw, t, type Key } from "./i18n";
import { rich } from "./i18n/rich";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, download, errorText, num, snippet, type LogEntry, type LogPage, type Persona } from "./api";
import { GeneralRows } from "./general/Rows";
import { flushLogs, logEvent } from "./log";
import { askForNotifications, notificationState, setNotifyPrefs, useNotifyPrefs } from "./notify";
import { Suggestions } from "./models/Suggestions";
import { Backups } from "./settings/Backups";
import { ChatTab } from "./settings/ChatTab";
import { Expert } from "./settings/Expert";
import { PhoneTab } from "./phone/Phone";
import { onOtherDevice } from "./phone/shell";
import { useFormFactor } from "./app/formfactor";
import { LanguageRow } from "./settings/Language";
import { ModelTab } from "./settings/ModelTab";
import { useSpot } from "./spot";
import { HOME_LAYOUTS, THEMES, type HomeLayoutId, type ThemeId } from "./theme";
import { Icon, Portrait } from "./ui";

export type SettingsTab = "general" | "chat" | "model" | "expert" | "suggestions" | "backups" | "phone" | "logs";

type Props = {
  tab: SettingsTab;
  /** The active persona; undefined while the library has not arrived. */
  persona: Persona | null | undefined;
  /** null while the settings have not arrived. */
  theme: ThemeId | null;
  onTheme: (id: ThemeId) => void;
  /** null while the settings have not arrived. */
  layout: HomeLayoutId | null;
  onLayout: (id: HomeLayoutId) => void;
};

/** Masks for the tab row's cut-off ends (alpha only, so they work in every theme). */
const FADES = {
  end: "linear-gradient(to right, #000 calc(100% - 44px), transparent)",
  start: "linear-gradient(to left, #000 calc(100% - 44px), transparent)",
  both: "linear-gradient(to right, transparent, #000 44px, #000 calc(100% - 44px), transparent)",
};

/** The line under the page's title. */
const subtitle = (tab: SettingsTab) => (tab === "phone" && onOtherDevice ? t("shell.settings.sub.connection") : t(`shell.settings.sub.${tab}`));

export function Settings({ tab, persona, theme, onTheme, layout, onLayout }: Props) {
  useSpot(tab);
  // On a phone the tab row scrolls sideways: keep the open tab in view (e.g. "Logs" at the far end).
  const tabsRef = useRef<HTMLElement>(null);
  useEffect(() => {
    const nav = tabsRef.current;
    const current = nav?.querySelector<HTMLElement>('[aria-current="page"]');
    if (!nav || !current) return;
    const left = current.offsetLeft - nav.offsetLeft;
    if (left < nav.scrollLeft || left + current.offsetWidth > nav.scrollLeft + nav.clientWidth) nav.scrollLeft = left - 48; // clear of the faded edge
  }, [tab]);
  // A row wider than the window fades out at each end that has more tabs behind it, so a cut-off
  // tab reads as "more this way" (phone, upright tablet, a narrow desktop window).
  const [more, setMore] = useState<keyof typeof FADES | null>(null);
  useEffect(() => {
    const nav = tabsRef.current;
    if (!nav) return;
    const check = () => {
      const start = nav.scrollLeft > 2;
      const end = nav.scrollLeft + nav.clientWidth < nav.scrollWidth - 2;
      setMore(start && end ? "both" : start ? "start" : end ? "end" : null);
    };
    check();
    nav.addEventListener("scroll", check, { passive: true });
    const sizes = new ResizeObserver(check);
    sizes.observe(nav);
    for (const a of nav.children) sizes.observe(a);
    return () => {
      nav.removeEventListener("scroll", check);
      sizes.disconnect();
    };
  }, []);
  return (
    <div className="chat-main">
      <header className="char-head settings-head">
        <div className="chat-head">
          <div className="t">
            <h1>{t("shell.settings.title")}</h1>
            <span>{subtitle(tab)}</span>
          </div>
        </div>
        <nav className="tabs" aria-label={t("shell.settings.tabs")} ref={tabsRef} data-more={more ?? undefined} style={more ? { maskImage: FADES[more], WebkitMaskImage: FADES[more] } : undefined}>
          <a href="#/settings" aria-current={tab === "general" ? "page" : undefined}>
            <Icon name="sliders" small />
            {t("shell.settings.tab.general")}
          </a>
          <a href="#/settings/chat" aria-current={tab === "chat" ? "page" : undefined}>
            <Icon name="chat" small />
            {t("shell.settings.tab.chat")}
          </a>
          <a href="#/settings/model" aria-current={tab === "model" ? "page" : undefined}>
            <Icon name="model" small />
            {t("shell.settings.tab.model")}
          </a>
          <a href="#/settings/expert" aria-current={tab === "expert" ? "page" : undefined}>
            <Icon name="bolt" small />
            {t("shell.settings.tab.expert")}
          </a>
          <a href="#/settings/suggestions" aria-current={tab === "suggestions" ? "page" : undefined}>
            <Icon name="download" small />
            {t("shell.settings.tab.suggestions")}
          </a>
          <a href="#/settings/backups" aria-current={tab === "backups" ? "page" : undefined}>
            <Icon name="memory" small />
            {t("shell.settings.tab.backups")}
          </a>
          <a href="#/settings/phone" aria-current={tab === "phone" ? "page" : undefined}>
            <Icon name="qr" small />
            {onOtherDevice ? t("shell.settings.tab.connection") : t("shell.settings.tab.phone")}
          </a>
          <a href="#/settings/logs" aria-current={tab === "logs" ? "page" : undefined}>
            <Icon name="list" small />
            {t("shell.settings.tab.logs")}
          </a>
        </nav>
      </header>
      {tab === "logs" ? (
        <LogsTab />
      ) : tab === "chat" ? (
        <ChatTab />
      ) : tab === "model" ? (
        <ModelTab />
      ) : tab === "expert" ? (
        <Expert />
      ) : tab === "suggestions" ? (
        <Suggestions />
      ) : tab === "backups" ? (
        <Backups />
      ) : tab === "phone" ? (
        <PhoneTab />
      ) : (
        <General persona={persona} theme={theme} onTheme={onTheme} layout={layout} onLayout={onLayout} />
      )}
    </div>
  );
}

type GeneralProps = {
  persona: Persona | null | undefined;
  theme: ThemeId | null;
  onTheme: (id: ThemeId) => void;
  layout: HomeLayoutId | null;
  onLayout: (id: HomeLayoutId) => void;
};

function General({ persona, theme, onTheme, layout, onLayout }: GeneralProps) {
  const form = useFormFactor();
  return (
    <div className="view">
      <div className="page set">
        <LanguageRow />
        <div className="set-row">
          <div className="set-label">
            <h2>{t("shell.general.persona.title")}</h2>
            <p>
              {t("shell.general.persona.text")}{" "}
              {form === "desktop" ? t("shell.general.persona.switchDesktop") : form === "tablet" ? t("shell.general.persona.switchTablet") : t("shell.general.persona.switchPhone")}
            </p>
          </div>
          <div className="set-panel" data-spot="name">
            {persona ? (
              <div className="set-persona">
                <Portrait who={persona} kind="persona" className="av" />
                <div className="sp-text">
                  <b>{persona.name}</b>
                  <span>{snippet(persona.description) || t("shell.general.persona.noDescription")}</span>
                </div>
              </div>
            ) : (
              <div className="set-persona" aria-busy="true">
                <span className="bar-ph" />
              </div>
            )}
            <p className="help">
              {rich("shell.general.persona.macro", { macro: <code className="macro">{"{{user}}"}</code>, name: persona?.name ?? "…" })}
            </p>
            <div className="btn-row">
              {persona && (
                <a className="btn sm" href={`#/personas/${persona.id}`}>
                  <Icon name="pencil" small />
                  {t("shell.general.persona.edit")}
                </a>
              )}
              <a className="btn sm ghost" href="#/personas">
                <Icon name="persona" small />
                {t("shell.persona.manage")}
              </a>
            </div>
          </div>
        </div>
        <div className="set-row">
          <div className="set-label">
            <h2>{t("shell.general.look.title")}</h2>
            <p>{t("shell.general.look.text")}</p>
          </div>
          <div className="set-panel">
            <span className="field-label">{t("shell.general.look.theme")}</span>
            <div className="themes" role="group" aria-label={t("shell.general.look.theme")} data-spot="theme">
              {THEMES.map((th) => (
                <button
                  key={th.id}
                  type="button"
                  className="theme-card"
                  aria-pressed={theme === th.id}
                  aria-label={t("shell.general.look.themeNamed", { name: th.name })}
                  disabled={theme === null}
                  onClick={() => theme !== th.id && onTheme(th.id)}
                >
                  {/* A small window in the theme's own colours: ground, a page on it, a line of text, the accent. */}
                  <span className="theme-sample" data-theme={th.id} aria-hidden="true">
                    <span className="ts-page">
                      <span className="ts-text">Aa</span>
                      <span className="ts-line" />
                      <span className="ts-line short" />
                    </span>
                    <span className="ts-accent" />
                  </span>
                  <span className="theme-name">
                    {th.name}
                    {th.id === "graphit" && <small>{t("shell.general.look.default")}</small>}
                    {theme === th.id && <Icon name="check" small />}
                  </span>
                </button>
              ))}
            </div>
            <span className="field-label">
              {t("shell.general.look.layout")}
              <small>{form === "desktop" ? t("shell.general.look.layoutHintDesktop") : t("shell.general.look.layoutHintOther")}</small>
            </span>
            <div className="layouts" role="group" aria-label={t("shell.general.look.layout")} data-spot="layout">
              {HOME_LAYOUTS.map((l) => (
                <button
                  key={l.id}
                  type="button"
                  className="layout-card"
                  aria-pressed={layout === l.id}
                  aria-label={t("shell.general.look.layoutNamed", { name: l.name })}
                  disabled={layout === null}
                  onClick={() => layout !== l.id && onLayout(l.id)}
                >
                  {/* A small sketch of the layout: where the big block, the lists and the tiles sit. */}
                  <span className={`layout-sample ls-${l.id}`} aria-hidden="true">
                    <i />
                    <i />
                    <i />
                    <i />
                  </span>
                  <span className="layout-text">
                    <span className="theme-name">
                      {l.name}
                      {l.id === "bisher" && <small>{t("shell.general.look.default")}</small>}
                      {layout === l.id && <Icon name="check" small />}
                    </span>
                    <span className="layout-note">{l.note}</span>
                  </span>
                </button>
              ))}
            </div>
          </div>
        </div>
        <GeneralRows>
          <AnswerHint />
        </GeneralRows>
      </div>
    </div>
  );
}

/** The hint when an answer is ready while the app is in the background. Kept on this device only. */
function AnswerHint() {
  const prefs = useNotifyPrefs();
  const [system, setSystem] = useState(notificationState);
  const systemNote =
    system === "granted"
      ? t("shell.hint.sys.granted")
      : system === "denied"
        ? t("shell.hint.sys.denied")
        : system === "default"
          ? t("shell.hint.sys.default")
          : "";
  return (
    <div className="set-row">
      <div className="set-label">
        <h2>{t("shell.hint.title")}</h2>
        <p>{t("shell.hint.text")}</p>
      </div>
      <div className="set-panel" data-spot="answer-hint">
        <label className="switch-row">
          <span className="sw-text">
            <b>{t("shell.hint.done")}</b>
            <span>{t("shell.hint.done.note")}</span>
          </span>
          <input
            type="checkbox"
            role="switch"
            className="switch"
            checked={prefs.on}
            onChange={(e) => {
              const on = e.target.checked;
              setNotifyPrefs({ on });
              if (on) void askForNotifications().then(setSystem);
            }}
          />
        </label>
        <label className={`switch-row${prefs.on ? "" : " off"}`}>
          <span className="sw-text">
            <b>{t("shell.hint.sound")}</b>
            <span>{t("shell.hint.sound.note")}</span>
          </span>
          <input
            type="checkbox"
            role="switch"
            className="switch"
            checked={prefs.sound}
            disabled={!prefs.on}
            onChange={(e) => setNotifyPrefs({ sound: e.target.checked })}
          />
        </label>
        {prefs.on && systemNote && <p className="help">{systemNote}</p>}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- logs

const FILTERS = [
  ["all", "system.logs.filter.all"],
  ["problems", "system.logs.filter.problems"],
  ["clicks", "system.logs.filter.clicks"],
  ["chat", "system.logs.filter.chat"],
  ["model", "system.logs.filter.model"],
  ["server", "system.logs.filter.server"],
] as const satisfies readonly (readonly [string, Key])[];
type Filter = (typeof FILTERS)[number][0];

const LEVEL_LABEL: Record<LogEntry["level"], Key> = { debug: "system.logs.level.debug", info: "system.logs.level.info", warn: "system.logs.level.warn", error: "system.logs.level.error" };
const SOURCE_LABEL: Record<string, Key> = { ui: "system.logs.source.ui", server: "system.logs.source.server", chat: "system.logs.source.chat", engine: "system.logs.source.engine" };

const PAGE = 200;
/** Rows kept on screen; live updates beyond that drop the oldest (they stay in the log). */
const KEEP = 2000;
const POLL_MS = 2000;

const byNewest = (a: LogEntry, b: LogEntry) => b.ts - a.ts || b.id - a.id;

const pad = (n: number, w = 2) => String(n).padStart(w, "0");
const clock = (ts: number) => {
  const d = new Date(ts);
  return [`${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`, `.${pad(d.getMilliseconds(), 3)}`] as const;
};
const dayKey = (ts: number) => new Date(ts).toDateString();
function dayLabel(ts: number): [string, string] {
  const d = new Date(ts);
  const full = dateFormat({ weekday: "long", day: "numeric", month: "long", year: "numeric" }).format(d);
  const days = Math.round((new Date().setHours(0, 0, 0, 0) - new Date(ts).setHours(0, 0, 0, 0)) / 86_400_000);
  if (days === 0) return [t("system.logs.today"), full];
  if (days === 1) return [t("system.logs.yesterday"), full];
  return [full, ""];
}

function query(params: Record<string, string | number | undefined>) {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== "") q.set(k, String(v));
  return `/logs?${q}`;
}

function LogsTab() {
  const [filter, setFilter] = useState<Filter>("all");
  const [search, setSearch] = useState("");
  const [q, setQ] = useState(""); // the search as sent, a moment after typing stops
  const [entries, setEntries] = useState<LogEntry[] | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [counts, setCounts] = useState({ total: 0, matching: 0 });
  const [error, setError] = useState("");
  const [open, setOpen] = useState<Set<number>>(new Set());
  const [confirming, setConfirming] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const maxId = useRef(0);
  const view = useRef({ filter, q }); // what the running poll belongs to

  useEffect(() => {
    const t = window.setTimeout(() => setQ(search.trim()), 250);
    return () => window.clearTimeout(t);
  }, [search]);

  const load = useCallback(async () => {
    const asked = { filter, q };
    view.current = asked;
    try {
      const page = await api<LogPage>(query({ filter, q, limit: PAGE }));
      if (view.current !== asked) return;
      maxId.current = page.max_id;
      setEntries(page.entries);
      setHasMore(page.has_more);
      setCounts({ total: page.total, matching: page.matching });
      setError("");
    } catch (e) {
      setError(errorText(e));
    }
  }, [filter, q]);

  useEffect(() => {
    setEntries(null);
    setOpen(new Set());
    void load();
  }, [load]);

  // Live: what was written since the last look, while this tab is open and the app in front. The
  // server merges a repeat only into the entry it wrote last (its counter "(×N)" goes up), so that
  // newest entry is asked for again each time and replaces the one on screen.
  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      if (document.visibilityState !== "visible") return;
      const asked = view.current;
      try {
        const page = await api<LogPage>(query({ filter: asked.filter, q: asked.q, after_id: Math.max(0, maxId.current - 1), limit: 500 }));
        if (cancelled || view.current !== asked) return;
        maxId.current = Math.max(maxId.current, page.max_id);
        setCounts({ total: page.total, matching: page.matching });
        if (page.entries.length === 0) return;
        setEntries((old) => {
          const shown = new Map((old ?? []).map((e) => [e.id, e]));
          const changed = page.entries.filter((e) => shown.get(e.id)?.message !== e.message);
          if (changed.length === 0) return old;
          const fresh = new Map(changed.map((e) => [e.id, e]));
          const kept = (old ?? []).map((e) => fresh.get(e.id) ?? e);
          const merged = [...changed.filter((e) => !shown.has(e.id)), ...kept].sort(byNewest);
          if (merged.length > KEEP) setHasMore(true);
          return merged.slice(0, KEEP);
        });
        setError("");
      } catch (e) {
        if (!cancelled) setError(errorText(e));
      }
    };
    const timer = window.setInterval(() => void tick(), POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const older = async () => {
    const last = entries?.[entries.length - 1];
    if (!last) return;
    const asked = view.current;
    setLoadingMore(true);
    try {
      const page = await api<LogPage>(query({ filter, q, limit: PAGE, before_ts: last.ts, before_id: last.id }));
      if (view.current !== asked) return;
      setEntries((old) => [...(old ?? []), ...page.entries]);
      setHasMore(page.has_more);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoadingMore(false);
    }
  };

  const exportFile = async () => {
    logEvent("info", "logs", "Log exportiert");
    await flushLogs(); // the latest clicks belong in the file
    const tz = -new Date().getTimezoneOffset();
    const scr = `${screen.width}x${screen.height}@${Math.round(devicePixelRatio * 100) / 100}`;
    const d = new Date();
    const two = (n: number) => String(n).padStart(2, "0");
    const stamp = `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}-${two(d.getHours())}${two(d.getMinutes())}`;
    try {
      const file = await download(`/logs/export?tz=${tz}&screen=${encodeURIComponent(scr)}`, {}, `lorelamp-logs-${stamp}.txt`);
      if (file !== null) setExported(file);
    } catch (e) {
      setError(t("system.logs.exportFailed", { error: errorText(e) }));
    }
  };
  const [exported, setExported] = useState("");
  useEffect(() => {
    if (!exported) return;
    const t = window.setTimeout(() => setExported(""), 4000);
    return () => window.clearTimeout(t);
  }, [exported]);

  const clear = async () => {
    setConfirming(false);
    try {
      await api<{ removed: number }>("/logs", { method: "DELETE" });
      logEvent("info", "logs", "Log geleert");
      await load();
    } catch (e) {
      setError(t("system.logs.clearFailed", { error: errorText(e) }));
    }
  };

  const toggle = (id: number) =>
    setOpen((s) => {
      const next = new Set(s);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  // Rows grouped under one heading per day.
  const days = useMemo(() => {
    const out: { key: string; ts: number; rows: LogEntry[] }[] = [];
    for (const e of entries ?? []) {
      const key = dayKey(e.ts);
      if (out[out.length - 1]?.key !== key) out.push({ key, ts: e.ts, rows: [] });
      out[out.length - 1].rows.push(e);
    }
    return out;
  }, [entries]);

  const filtered = filter !== "all" || q !== "";

  return (
    <div className="view">
      <div className="page logs">
        <div className="log-tools">
          <div className="seg" role="group" aria-label={t("system.logs.filter")} data-spot="logs">
            {FILTERS.map(([id, label]) => (
              <button key={id} type="button" aria-pressed={filter === id} onClick={() => setFilter(id)}>
                {raw(label)}
              </button>
            ))}
          </div>
          <label className="search">
            <Icon name="search" small />
            <input
              className="input"
              type="search"
              value={search}
              placeholder={t("system.logs.search")}
              aria-label={t("system.logs.search")}
              onChange={(e) => setSearch(e.target.value)}
            />
          </label>
          <div className="log-actions">
            <span className="log-count num">
              {filtered
                ? rich("system.logs.countOf", { matching: <span className="data">{num(counts.matching)}</span>, total: num(counts.total), n: counts.total })
                : rich("system.logs.count", { total: <span className="data">{num(counts.total)}</span>, n: counts.total })}
            </span>
            {confirming ? (
              <span className="confirm">
                <button className="btn sm danger" onClick={() => void clear()}>
                  {t("system.logs.clearAll")}
                </button>
                <button className="btn sm ghost" onClick={() => setConfirming(false)}>
                  {t("system.cancel")}
                </button>
              </span>
            ) : (
              <>
                <button className="btn sm" onClick={() => void exportFile()} disabled={counts.total === 0} data-spot="export">
                  <Icon name="download" small />
                  {t("system.logs.export")}
                </button>
                <button className="btn sm ghost" onClick={() => setConfirming(true)} disabled={counts.total === 0}>
                  <Icon name="trash" small />
                  {t("system.logs.clear")}
                </button>
              </>
            )}
          </div>
        </div>
        <p className="help log-confirm">{t("system.logs.keeps")}</p>
        {exported && (
          <p className="help log-confirm" role="status">
            {t("system.logs.saved", { file: exported })}
          </p>
        )}
        {confirming && (
          <p className="help log-confirm">{t("system.logs.clearAsk")}</p>
        )}
        {error && <p className="bad log-error">{error}</p>}

        {entries === null ? (
          !error && <p className="empty-line">{t("system.logs.loading")}</p>
        ) : entries.length === 0 ? (
          <p className="empty-line">{filtered ? t("system.logs.noneFound") : t("system.logs.none")}</p>
        ) : (
          <>
            {days.map((day) => {
              const [label, full] = dayLabel(day.ts);
              return (
                <section key={day.key} aria-label={label}>
                  <h2 className="log-day">
                    {label}
                    {full && <small>{full}</small>}
                  </h2>
                  <ul className="log-list">
                    {day.rows.map((e) => (
                      <LogRow key={e.id} e={e} open={open.has(e.id)} onToggle={() => toggle(e.id)} />
                    ))}
                  </ul>
                </section>
              );
            })}
            {hasMore && (
              <div className="log-more">
                <button className="btn sm" onClick={() => void older()} disabled={loadingMore}>
                  {loadingMore ? t("system.logs.loading") : t("system.logs.older")}
                </button>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function LogRow({ e, open, onToggle }: { e: LogEntry; open: boolean; onToggle: () => void }) {
  const [hms, ms] = clock(e.ts);
  const hasDetail = e.data !== undefined || e.device || e.chat_id !== null || e.character_id !== null;
  return (
    <li className={`log-row lv-${e.level}${open ? " open" : ""}`}>
      <button className="log-main" aria-expanded={hasDetail ? open : undefined} onClick={onToggle} data-log-label="Logeintrag auf-/zuklappen">
        <time className="num" dateTime={new Date(e.ts).toISOString()}>
          {hms}
          <span className="log-ms">{ms}</span>
        </time>
        <span className="log-lv">{e.level in LEVEL_LABEL ? raw(LEVEL_LABEL[e.level]) : e.level}</span>
        <span className="log-src">
          {e.source in SOURCE_LABEL ? raw(SOURCE_LABEL[e.source]) : e.source} · {e.category}
        </span>
        <span className="log-msg">{e.message}</span>
      </button>
      {open && (
        <div className="log-detail">
          <dl className="log-facts">
            <div>
              <dt>{t("system.logs.time")}</dt>
              <dd className="num">
                {dateFormat({ dateStyle: "medium", timeStyle: "medium" }).format(e.ts)}
                {ms}
              </dd>
            </div>
            {e.device && (
              <div>
                <dt>{t("system.logs.device")}</dt>
                <dd>{e.device}</dd>
              </div>
            )}
            {e.chat_id !== null && (
              <div>
                <dt>{t("system.logs.chat")}</dt>
                <dd>
                  <a href={`#/chat/${e.chat_id}`} className="num">
                    {e.chat_id}
                  </a>
                </dd>
              </div>
            )}
            {e.character_id !== null && (
              <div>
                <dt>{t("system.logs.figure")}</dt>
                <dd>
                  <a href={`#/char/${e.character_id}/chats`} className="num">
                    {e.character_id}
                  </a>
                </dd>
              </div>
            )}
            <div>
              <dt>{t("system.logs.id")}</dt>
              <dd className="num">{e.id}</dd>
            </div>
          </dl>
          {e.data !== undefined && <pre className="code">{JSON.stringify(e.data, null, 2)}</pre>}
        </div>
      )}
    </li>
  );
}
