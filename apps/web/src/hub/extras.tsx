// What every start layout offers besides "carry on": the model's state with the one thing to do about
// it, the first steps of a new installation, a new chat or a star on every figure, the figures that
// were imported but not played yet, and the switch between the five layouts.

import { createContext, lazy, Suspense, useContext, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { api, modelFacts, modelName, when, type CharInfo, type Persona, type Timings } from "../api";
import { t } from "../i18n";
import { setModelLoaded } from "../app/engineLoad";
import { problemOf, restartModel } from "../app/engineProblem";
import type { Command } from "../commands";
import { HOME_LAYOUTS, type HomeLayoutId } from "../theme";
import { hueStyle, Icon, Portrait } from "../ui";
import { setFavorite } from "./act";
import type { EngineView } from "./blocks";
import "./hub.css";

// Only a new installation needs it (with the PC scan and engine downloads behind it): its own chunk.
const EngineSetup = lazy(() => import("./EngineSetup").then((m) => ({ default: m.EngineSetup })));

/** What the pieces below need from the app, so the layouts do not hand it through every block. */
export type HubTools = { onNewChat: (characterId: number) => void; onImport: () => void; commands: Command[] };
const Tools = createContext<HubTools | null>(null);
export const HubToolsProvider = Tools.Provider;
function useTools(): HubTools {
  const tools = useContext(Tools);
  if (!tools) throw new Error("start screen piece outside of Hub");
  return tools;
}
/** Runs a quick action (Ctrl+K entry) by its id; the app owns what it does. */
const runCommand = (commands: Command[], id: string) => commands.find((c) => c.id === id)?.run?.();

// ---------- The model ----------

export type ModelKind = "ready" | "idle" | "loading" | "connecting" | "setup" | "stopped" | "port" | "down";
export type ModelState = {
  kind: ModelKind;
  /** A few words: what is the case. */
  title: string;
  /** One sentence more, where there is something to say. */
  line: string;
  /** Needs the user: no chat will get an answer until this is solved. */
  blocked: boolean;
};

/** What the start screen says about the model and whether the user has to do something. */
export function modelState(engine: EngineView): ModelState {
  const info = engine.info;
  const state = (kind: ModelKind, title: string, line = ""): ModelState => ({ kind, title, line, blocked: kind === "setup" || kind === "stopped" || kind === "port" || kind === "down" });
  if (engine.status === "connecting") return state("connecting", t("system.model.connecting"));
  if (engine.status === "ready") return state("ready", t("system.model.ready"));
  if (engine.status === "idle") return state("idle", t("system.model.idle"), t("system.model.idle.line"));
  const problem = problemOf(info);
  if (problem?.code === "gave_up") return state("stopped", t("system.model.stopped"), t("system.model.stopped.line", { n: problem.crashes }));
  if (problem?.code === "port_busy") return state("port", t("system.model.port"), t("system.model.port.line", { port: String(problem.port) }));
  if (info?.managed && !info.profile) return state("setup", t("system.model.setup"), t("system.model.setup.line"));
  if (engine.status === "loading") return state("loading", t("system.model.loading"));
  return state("down", t("system.model.down"), info?.managed ? t("system.model.down.managed") : t("system.model.down.foreign"));
}

/** The one thing to do about the model's state, as a button or a link; nothing while all is well. */
export function ModelAct({ state, primary }: { state: ModelState; primary?: boolean }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cls = `btn sm ${primary ? "primary" : "solid"} hx-model-act`;
  const press = async (what: () => Promise<string | null>) => {
    setBusy(true);
    setError((await what()) ?? "");
    setBusy(false);
  };
  let act: ReactNode = null;
  if (state.kind === "idle") {
    act = (
      <button type="button" className={cls} disabled={busy} onClick={() => void press(() => setModelLoaded(true))} data-log-label="Modell laden">
        {t("system.model.act.load")}
      </button>
    );
  } else if (state.kind === "stopped") {
    act = (
      <button
        type="button"
        className={cls}
        disabled={busy}
        onClick={() => void press(async () => {
          const r = await restartModel();
          return typeof r === "string" ? r : null;
        })}
        data-log-label="Modell neu starten"
      >
        {t("system.model.act.restart")}
      </button>
    );
  } else if (state.kind === "setup") {
    act = (
      <a className={cls} href="#/settings/suggestions" data-log-label="Modell einrichten">
        {t("system.model.act.setup")}
      </a>
    );
  } else if (state.kind === "port") {
    act = (
      <a className={cls} href="#/settings/model" data-log-label="Port ändern">
        {t("system.model.act.port")}
      </a>
    );
  } else if (state.kind === "down") {
    act = (
      <a className={cls} href="#/settings/model" data-log-label="Modell prüfen">
        {t("system.model.act.check")}
      </a>
    );
  }
  if (!act) return null;
  return (
    <>
      {act}
      {error && (
        <span className="hx-model-error" role="alert">
          {error}
        </span>
      )}
    </>
  );
}

const DOT: Record<ModelKind, string> = { ready: "ready", idle: "idle", loading: "loading", connecting: "offline", setup: "todo", stopped: "offline", port: "offline", down: "offline" };

/** The model's facts in the words the sidebar's status uses: quant and context, whether its start
 *  options were measured or estimated, the newest speed. One wording for both places. */
export function modelLine(engine: EngineView, timings: Timings | null): string {
  const profile = engine.info?.profile;
  return [
    modelFacts(engine.info?.model),
    profile ? (profile.measured ? t("shell.pill.measured") : t("shell.pill.estimated")) : "",
    timings ? t("shell.pill.tokens", { n: Math.round(timings.tok_s) }) : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/** The model in one line: its state, its name, the newest speed, and what to do if it does not run. */
export function ModelStrip({ engine, timings }: { engine: EngineView; timings: Timings | null }) {
  const state = modelState(engine);
  const m = engine.info?.model;
  const name = modelName(m);
  const calm = state.kind === "ready";
  const profile = engine.info?.profile;
  return (
    <section className={`hx-strip${state.blocked ? " blocked" : calm ? " calm" : ""}`} aria-label={t("system.hub.model")}>
      <a className="hx-strip-open" href="#/settings/model" title={calm && profile ? (profile.measured ? t("shell.pill.measuredTitle") : t("shell.pill.estimatedTitle")) : undefined} data-log-label="Modell öffnen">
        <i className={`dot ${DOT[state.kind]}`} />
        <span className="hx-strip-text">
          <b>{calm && name ? name : state.title}</b>
          <span className="num">
            {calm
              ? modelLine(engine, timings) || state.title
              : state.kind === "idle" || state.kind === "loading"
                ? name || state.line
                : state.line}
          </span>
        </span>
      </a>
      {/* What blocks every chat has its own call above the page (ModelCall) or is the first step. */}
      {state.kind === "idle" && <ModelAct state={state} />}
    </section>
  );
}

/** Above everything while no chat can get an answer: what is wrong and the way out. */
export function ModelCall({ engine, note }: { engine: EngineView; note: string }) {
  const state = modelState(engine);
  if (!state.blocked) return null;
  return (
    <div className="hx-call" role="alert">
      <span className="hx-call-ic">
        <Icon name={state.kind === "setup" ? "model" : "alert"} small />
      </span>
      <p>
        <b>{state.title}.</b> {state.kind === "down" && note ? note : state.line}
      </p>
      <ModelAct state={state} primary />
    </div>
  );
}

// ---------- Figures: a new chat and the star ----------

/** One press starts a fresh chat with the figure. */
export function NewChat({ c, label, className = "" }: { c: CharInfo; label?: string; className?: string }) {
  const { onNewChat } = useTools();
  return (
    <button
      type="button"
      className={`${label ? "btn sm solid" : "hx-round"} hx-new ${className}`}
      onClick={() => onNewChat(c.id)}
      aria-label={t("system.hub.newChatWith", { name: c.name })}
      title={label ? undefined : t("system.hub.newChatWith", { name: c.name })}
      data-log-label="Neuer Chat mit der Figur"
    >
      <Icon name="plus" small />
      {label && <span>{label}</span>}
    </button>
  );
}

/** The library's star, settable from the start screen too. Favourites come first everywhere here. */
export function Star({ c, className = "" }: { c: CharInfo; className?: string }) {
  const [pending, setPending] = useState<boolean | null>(null);
  const on = pending ?? c.favorite;
  // The library's answer has arrived: the server's value counts again.
  if (pending !== null && pending === c.favorite) setPending(null);
  const toggle = async () => {
    const next = !on;
    setPending(next);
    if (await setFavorite(c.id, next)) setPending(null);
  };
  return (
    <button
      type="button"
      className={`hx-round hx-star${on ? " on" : ""} ${className}`}
      aria-pressed={on}
      aria-label={t("system.hub.fav", { name: c.name })}
      title={on ? t("system.hub.fav.on") : t("system.hub.fav.off")}
      onClick={() => void toggle()}
      data-log-label="Favorit"
    >
      <Icon name="star" small />
    </button>
  );
}

/** A figure as one row: open it, star it, start a chat. `fresh`: imported but never played. */
export function FigureLine({ c, fresh }: { c: CharInfo; fresh?: boolean }) {
  return (
    <li className="hx-line" style={hueStyle(c.name)}>
      <a className="hx-line-open" href={`#/char/${c.id}/chats`} data-log-label="Figur öffnen">
        <Portrait who={c} className="av" />
        <span className="hx-line-text">
          <b>{c.name}</b>
          <span className="num">{fresh ? t("system.hub.imported", { when: when(c.created_at) }) : c.blurb || (c.chats === 0 ? t("system.hub.noChat") : t("system.hub.chats", { n: c.chats }))}</span>
        </span>
      </a>
      {!fresh && <Star c={c} />}
      <NewChat c={c} label={fresh ? t("system.hub.firstChat") : undefined} />
    </li>
  );
}

export function FigureLines({ list, fresh, label }: { list: CharInfo[]; fresh?: boolean; label: string }) {
  return (
    <ul className="hx-lines" aria-label={label}>
      {list.map((c) => (
        <FigureLine key={c.id} c={c} fresh={fresh} />
      ))}
    </ul>
  );
}

/** Imported, not played yet: what is new on the shelf, each one press away from its first chat. */
export function Fresh({ list, className = "", id }: { list: CharInfo[]; className?: string; id: string }) {
  if (list.length === 0) return null;
  return (
    <section className={`hx-fresh ${className}`} aria-labelledby={id}>
      <h2 className="hx-head" id={id}>
        {t("system.hub.fresh")}
        <span className="hx-head-note">{t("system.hub.fresh.note")}</span>
      </h2>
      <FigureLines list={list} fresh label={t("system.hub.fresh")} />
    </section>
  );
}

// ---------- The first steps ----------

/** The name the user plays under (the active persona's), or null while it is not known. "User" is the
 *  name a new installation starts with, so it counts as not chosen yet. */
export const DEFAULT_USER_NAME = "User";
function useActivePersona(again: unknown): { id: number; name: string } | null {
  const [persona, setPersona] = useState<{ id: number; name: string } | null>(null);
  useEffect(() => {
    let live = true;
    api<{ personas: Persona[]; active_persona_id: number }>("/personas")
      .then((r) => {
        const p = r.personas.find((x) => x.id === r.active_persona_id);
        if (live && p) setPersona({ id: p.id, name: p.name });
      })
      // Without an answer the step simply stays open; the other three do not depend on it.
      .catch(() => {});
    return () => {
      live = false;
    };
    // `again`: the library changed (a name set in the tour or the persona editor arrives with it).
  }, [again]);
  return persona;
}

const FIRST_STEPS = 4;

/** Until the first chat exists: the four steps to it (the same ones the welcome tour walks through),
 *  each with its real state and its way on. */
export function FirstSteps({ engine, characters, className = "" }: { engine: EngineView; characters: CharInfo[]; className?: string }) {
  const { onImport, commands } = useTools();
  const model = modelState(engine);
  // Before `/engine` has answered (it can take a second or two) nothing is known about the model: the
  // step waits instead of calling a model "set up" that may not exist.
  const known = engine.info != null && model.kind !== "connecting";
  const modelDone = known && (model.kind === "ready" || model.kind === "idle" || model.kind === "loading");
  const figures = characters.length;
  const persona = useActivePersona(characters);
  const userName = persona?.name.trim() ?? "";
  const named = userName !== "" && userName !== DEFAULT_USER_NAME;
  const done = Number(modelDone) + Number(figures > 0) + Number(named);
  const left = FIRST_STEPS - done;
  const m = modelName(engine.info?.model);
  return (
    <section className={`hx-first ${className}`} aria-labelledby="hx-first">
      <div className="hx-first-head">
        <h2 id="hx-first">{done === 0 ? t("system.first.title") : t("system.first.titleLeft", { n: left })}</h2>
        <p>
          {t("system.first.lead")} <span className="num">{t("system.first.done", { done, all: FIRST_STEPS })}</span>
        </p>
        <span className="hx-progress" role="img" aria-label={t("system.first.doneAria", { done, all: FIRST_STEPS })}>
          <i style={{ width: `${(done / FIRST_STEPS) * 100}%` }} />
        </span>
      </div>
      <ol className="hx-steps">
        <li className={`hx-step${modelDone ? " done" : " now"}`}>
          <StepMark done={modelDone} n={1} />
          <div className="hx-step-text">
            <h3>{t("system.first.model")}</h3>
            <p>{modelDone ? (m ? t("system.first.model.named", { name: m }) : t("system.first.model.some")) : !known ? t("system.first.model.connecting") : `${model.title}.`}</p>
          </div>
          {!known && <EngineWait />}
          {known && !modelDone && (
            <Suspense fallback={<EngineWait />}>
              <EngineSetup
                note={model.line}
                acts={() => (
                  <>
                    <ModelAct state={model} primary />
                    {model.kind === "setup" && (
                      <a className="btn sm ghost" href="#/settings/model" data-log-label="Eigenes Modell wählen">
                        {t("system.first.ownFile")}
                      </a>
                    )}
                  </>
                )}
              />
            </Suspense>
          )}
        </li>
        <li className={`hx-step${figures > 0 ? " done" : modelDone ? " now" : ""}`}>
          <StepMark done={figures > 0} n={2} />
          <div className="hx-step-text">
            <h3>{t("system.first.figure")}</h3>
            <p>{figures > 0 ? t("system.first.figure.there", { n: figures }) : t("system.first.figure.how")}</p>
          </div>
          {figures === 0 && (
            <div className="hx-step-acts">
              <button type="button" className={`btn sm ${modelDone ? "primary" : "solid"}`} onClick={onImport} data-log-label="Karte importieren">
                <Icon name="upload" small />
                {t("system.hub.import")}
              </button>
              <button type="button" className="btn sm ghost" onClick={() => runCommand(commands, "import-sillytavern")} data-log-label="Aus SillyTavern übernehmen">
                {t("system.first.fromSt")}
              </button>
            </div>
          )}
        </li>
        <li className={`hx-step${named ? " done" : figures > 0 && modelDone ? " now" : ""}`}>
          <StepMark done={named} n={3} />
          <div className="hx-step-text">
            <h3>{t("system.first.name")}</h3>
            <p>{named ? t("system.first.name.set", { name: userName }) : t("system.first.name.how")}</p>
          </div>
          {persona && !named && (
            <div className="hx-step-acts">
              <a className="btn sm solid" href={`#/personas/${persona.id}`} data-log-label="Namen festlegen">
                <Icon name="persona" small />
                {t("system.first.name.act")}
              </a>
            </div>
          )}
        </li>
        <li className={`hx-step${figures > 0 ? " now" : ""}`}>
          <StepMark done={false} n={4} />
          <div className="hx-step-text">
            <h3>{t("system.first.chat")}</h3>
            <p>{figures > 0 ? t("system.first.chat.pick") : t("system.first.chat.later")}</p>
          </div>
          {figures > 0 && (
            <div className="hx-step-acts hx-step-figs">
              {characters.slice(0, 3).map((c) => (
                <FirstChat key={c.id} c={c} />
              ))}
            </div>
          )}
        </li>
      </ol>
    </section>
  );
}

/** Holds the place of the model step's buttons while it is not known yet which ones it gets, so the
 *  step does not grow under the user's eyes. */
export function EngineWait() {
  return (
    <div className="hx-step-acts hx-wait" aria-hidden="true">
      <i />
      <i />
    </div>
  );
}

function StepMark({ done, n }: { done: boolean; n: number }) {
  return (
    <span className="hx-step-mark num" aria-hidden="true">
      {done ? <Icon name="check" small /> : n}
    </span>
  );
}

function FirstChat({ c }: { c: CharInfo }) {
  const { onNewChat } = useTools();
  return (
    <button type="button" className="hx-first-chat" style={hueStyle(c.name)} onClick={() => onNewChat(c.id)} aria-label={t("system.hub.newChatWith", { name: c.name })} data-log-label="Neuer Chat mit der Figur">
      <Portrait who={c} className="av" />
      <b>{c.name}</b>
      <Icon name="next" small />
    </button>
  );
}

// ---------- The layout switch ----------

/** Each layout as a few blocks on a 30×22 sheet: [x, y, width, height]. */
const SKETCH: Record<HomeLayoutId, [number, number, number, number][]> = {
  bisher: [[2, 2, 16, 8], [2, 12, 16, 3], [2, 17, 16, 3], [20, 2, 8, 8], [20, 12, 8, 8]],
  dashboard: [[2, 2, 26, 4], [2, 8, 17, 6], [2, 16, 17, 4], [21, 8, 7, 12]],
  bento: [[2, 2, 16, 10], [20, 2, 8, 4], [20, 8, 8, 4], [2, 14, 8, 6], [12, 14, 6, 6], [20, 14, 8, 6]],
  buehne: [[0, 0, 30, 10], [2, 13, 6, 7], [10, 13, 6, 7], [18, 13, 6, 7], [26, 13, 4, 7]],
  kommando: [[2, 2, 17, 4], [21, 2, 7, 7], [2, 8, 17, 2], [2, 12, 13, 8], [17, 12, 11, 8]],
};

function Sketch({ id }: { id: HomeLayoutId }) {
  return (
    <svg className="hx-sketch" viewBox="0 0 30 22" aria-hidden="true">
      {SKETCH[id].map(([x, y, w, h], i) => (
        <rect key={i} x={x} y={y} width={w} height={h} rx="1.2" />
      ))}
    </svg>
  );
}

const EDGE = 8;
const MENU_WIDTH = 340;

/** The five start layouts, one press away on the start screen itself. The choice is the app's setting
 *  "Start-Layout" (the same on every device); it goes through the quick action of that name. */
export function LayoutSwitch({ layout, className = "" }: { layout: HomeLayoutId; className?: string }) {
  const { commands } = useTools();
  const [open, setOpen] = useState(false);
  const [sheet, setSheet] = useState(false);
  const [place, setPlace] = useState<CSSProperties | undefined>(undefined);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const current = HOME_LAYOUTS.find((l) => l.id === layout) ?? HOME_LAYOUTS[0];

  useLayoutEffect(() => {
    if (!open) return;
    const fit = () => {
      const phone = matchMedia("(max-width: 767px)").matches;
      setSheet(phone);
      const r = button.current?.getBoundingClientRect();
      if (phone || !r) return setPlace(undefined);
      const width = Math.min(MENU_WIDTH, window.innerWidth - 2 * EDGE);
      const left = Math.min(Math.max(r.right - width, EDGE), window.innerWidth - EDGE - width);
      setPlace({ left, width, top: r.bottom + 6, maxHeight: Math.max(200, window.innerHeight - r.bottom - 6 - EDGE) });
    };
    fit();
    const close = () => setOpen(false);
    window.addEventListener("resize", fit);
    window.addEventListener("hashchange", close);
    return () => {
      window.removeEventListener("resize", fit);
      window.removeEventListener("hashchange", close);
    };
  }, [open]);
  useEffect(() => {
    if (open) menu.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
  }, [open]);

  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };
  const onKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const all = [...(menu.current?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? [])];
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
      close(true);
    } else if (e.key === "Tab") close(false);
  };

  return (
    <>
      <button
        ref={button}
        type="button"
        className={`btn sm solid hx-switch ${className}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("system.layout.switchNow", { name: current.name })}
        title={t("system.layout.switch")}
        onClick={() => setOpen((o) => !o)}
        data-log-label="Start-Layout wechseln"
      >
        <Icon name="layout" small />
        <span>{current.name}</span>
        <Icon name="down" small />
      </button>
      {open &&
        createPortal(
          <>
            <div className={`pm-scrim${sheet ? " sheet" : ""}`} onPointerDown={() => close(false)} aria-hidden="true" />
            <div ref={menu} className={`pm hx-menu${sheet ? " sheet" : " down"}`} role="menu" aria-label={t("system.layout.title")} style={place} onKeyDown={onKey}>
              {sheet && <i className="pm-grab" aria-hidden="true" />}
              <div className="pm-head">
                {t("system.layout.title")}
                <small>{t("system.layout.note")}</small>
              </div>
              <div className="pm-list">
                {HOME_LAYOUTS.map((l) => (
                  <button
                    key={l.id}
                    type="button"
                    role="menuitemradio"
                    aria-checked={l.id === layout}
                    className="pm-item"
                    tabIndex={-1}
                    onClick={() => {
                      close(true);
                      if (l.id !== layout) runCommand(commands, `layout-${l.id}`);
                    }}
                    data-log-label="Start-Layout wählen"
                  >
                    <Sketch id={l.id} />
                    <span className="pm-text">
                      <b>{l.name}</b>
                      <small>{l.note}</small>
                    </span>
                    {l.id === layout && <Icon name="check" small />}
                  </button>
                ))}
              </div>
            </div>
          </>,
          document.body,
        )}
    </>
  );
}

// ---------- What is planned ----------

/** What is planned but not built, as one quiet line at the foot of the page. Never links. */
export function SoonLine({ items }: { items: { id: string; name: string; note: string }[] }) {
  return (
    <footer className="hx-soon">
      <span id="hx-soon">{t("system.hub.soon")}</span>
      <ul aria-labelledby="hx-soon">
        {items.map((s) => (
          <li key={s.id} title={s.note} aria-label={t("system.hub.soon.item", { name: s.name })}>
            {s.name}
          </li>
        ))}
      </ul>
    </footer>
  );
}
