// The welcome tour itself (its own chunk; TutorialHost.tsx loads it only while the tour is on). A modal
// window over the app: on a desktop or tablet a sheet in the middle, on a phone the whole screen.
// Five places: the welcome with the language, then the model, a character, the user's name and the
// first chat. Each step shows its real state and the way on; every one can be skipped, "Später" ends
// the tour. A link into the app (model suggestions, import, the persona) puts the tour aside as a small
// button at the edge, so the user can do the step there and come back.

import { lazy, Suspense, useEffect, useId, useRef, useState, type CSSProperties, type FormEvent, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { errorText, modelName, postJson, type Library } from "../api";
import type { EngineView } from "../hub/blocks";
import { DEFAULT_USER_NAME, EngineWait, ModelAct, modelState } from "../hub/extras";
import { lang, LANGS, setLanguage, t, type LangId } from "../i18n";
import { logEvent } from "../log";
import { onOtherDevice } from "../phone/shell";
import { BrandMark, hueStyle, Icon, Portrait, type IconName } from "../ui";
import { dockTour, goTo, STEPS, type Step } from "./store";
import "./tutorial.css";

// Asked for as soon as the tour loads, so the model step does not open empty and grow a moment later.
const engineSetup = import("../hub/EngineSetup");
const EngineSetup = lazy(() => engineSetup.then((m) => ({ default: m.EngineSetup })));

const REPO = "https://github.com/Lampe332/lorelamp";
/** The steps after the welcome, counted as "Schritt n von 4". */
const TASKS = STEPS.slice(1);

/** A language picked here redraws the whole app; this tells the new tour to put the focus back on it. */
let focusLanguage: LangId | null = null;

export type TourProps = { step: Step; docked: boolean; library: Library | null; engine: EngineView };

export function Tour(props: TourProps) {
  return props.docked ? <Dock step={props.step} /> : <Sheet {...props} />;
}

/** The tour put aside: one button at the edge that brings it back. */
function Dock({ step }: { step: Step }) {
  const n = TASKS.indexOf(step as (typeof TASKS)[number]) + 1;
  return (
    <button type="button" className="tu-dock" onClick={() => dockTour(false)} data-log-label="Einführung fortsetzen">
      <BrandMark className="tu-dock-mark" />
      <span className="tu-dock-text">
        <b>{t("tutorial.dock")}</b>
        {n > 0 && <span className="num">{t("tutorial.step", { step: String(n), all: String(TASKS.length) })}</span>}
      </span>
    </button>
  );
}

function Sheet({ step, library, engine }: TourProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [problem, setProblem] = useState("");
  // Which way the last move went, so a step slides in from the side it comes from.
  const [dir, setDir] = useState<1 | -1>(1);
  const index = STEPS.indexOf(step);
  const task = index; // 1…4 after the welcome

  useEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
    return () => d?.close();
  }, []);

  // A new step: its heading takes the focus, so a screen reader reads where the user is now.
  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    const lang = step === "welcome" ? focusLanguage : null;
    focusLanguage = null;
    const target = d.querySelector<HTMLElement>(step !== "welcome" ? ".tu-title" : lang ? `input[value="${lang}"]` : ".tu-start");
    // After the frame: the window has just opened and moved the focus itself.
    const frame = requestAnimationFrame(() => target?.focus());
    return () => cancelAnimationFrame(frame);
  }, [step]);

  const move = (to: Step | "done", how: string) => {
    logEvent("info", "settings", how, { data: { from: step, to } });
    setDir(to !== "done" && STEPS.indexOf(to) < index ? -1 : 1);
    setProblem("");
    void goTo(to).then((bad) => bad && setProblem(t("tutorial.failed", { error: bad })));
  };
  const next = () => move(STEPS[index + 1] ?? "done", "Einführung weiter");
  const back = () => move(STEPS[index - 1] ?? "welcome", "Einführung zurück");
  const later = () => move("done", "Einführung später");

  // A link into the app puts the tour aside while the user is there (the button at the edge brings it back).
  const onClick = (e: MouseEvent) => {
    const a = (e.target as Element).closest("a");
    if (a?.getAttribute("href")?.startsWith("#/")) dockTour(true);
  };

  const figures = library?.characters ?? [];
  const modelDone = modelSetUp(engine) === true;

  let body: ReactNode;
  let foot: ReactNode;
  if (step === "welcome") {
    body = <Welcome />;
    foot = (
      <>
        <p className="tu-foot-note">{t("tutorial.welcome.again")}</p>
        <button type="button" className="btn primary tu-start" onClick={next} data-log-label="Einführung beginnen">
          {t("tutorial.welcome.start")}
          <Icon name="next" small />
        </button>
      </>
    );
  } else if (step === "model") {
    body = <ModelStep engine={engine} done={modelDone} />;
    foot = <Moves back={back} next={modelDone ? next : null} skip={modelDone ? null : next} />;
  } else if (step === "figure") {
    body = <FigureStep library={library} />;
    foot = <Moves back={back} next={figures.length > 0 ? next : null} skip={figures.length > 0 ? null : next} />;
  } else if (step === "persona") {
    body = <PersonaStep library={library} onDone={next} onProblem={setProblem} />;
    foot = (
      <Moves back={back} skip={next}>
        <button type="submit" form="tu-persona" className="btn primary" disabled={!library}>
          {t("tutorial.next")}
        </button>
      </Moves>
    );
  } else {
    body = <ChatStep library={library} quiet={!modelDone} onFigure={() => move("figure", "Einführung zurück")} onStarted={() => move("done", "Einführung: erster Chat")} onProblem={setProblem} />;
    foot = (
      <Moves back={back}>
        <button type="button" className="btn primary" onClick={() => move("done", "Einführung fertig")}>
          {t("tutorial.finish")}
        </button>
      </Moves>
    );
  }

  const onKey = (e: ReactKeyboardEvent) => {
    // Escape puts the tour aside instead of ending it; "Später" is the way out for good.
    if (e.key === "Escape") {
      e.preventDefault();
      dockTour(true);
    }
  };

  return (
    <dialog ref={dialog} className="tu" aria-labelledby="tu-title" aria-describedby="tu-lead" onCancel={(e) => e.preventDefault()} onKeyDown={onKey} onClick={onClick}>
      <div className={`tu-sheet tu-at-${step}`}>
        <header className="tu-head">
          {task > 0 ? (
            <div className="tu-progress">
              <ol className="tu-dots" aria-label={t("tutorial.steps")}>
                {TASKS.map((s, i) => (
                  <li key={s} className={i + 1 < task ? "past" : i + 1 === task ? "now" : ""} aria-current={i + 1 === task ? "step" : undefined}>
                    <span className="sr">{t("tutorial.step", { step: String(i + 1), all: String(TASKS.length) })}</span>
                  </li>
                ))}
              </ol>
              <span className="tu-count num" aria-hidden="true">
                {t("tutorial.step", { step: String(task), all: String(TASKS.length) })}
              </span>
            </div>
          ) : (
            <span />
          )}
          <button type="button" className="btn sm ghost tu-later" onClick={later} data-log-label="Einführung später">
            {t("tutorial.later")}
          </button>
        </header>
        <div className="tu-body" key={step} style={{ "--tu-dir": dir } as CSSProperties}>
          {body}
        </div>
        {problem && (
          <p className="bad tu-problem" role="alert">
            {problem}
          </p>
        )}
        <footer className="tu-foot">{foot}</footer>
      </div>
    </dialog>
  );
}

/** The step's moves: back at the left, skip and the way on at the right. */
function Moves({ back, skip, next, children }: { back: () => void; skip?: (() => void) | null; next?: (() => void) | null; children?: ReactNode }) {
  return (
    <>
      <button type="button" className="btn ghost tu-back" onClick={back} data-log-label="Einführung zurück">
        <Icon name="back" small />
        {t("tutorial.back")}
      </button>
      <span className="tu-moves">
        {skip && (
          <button type="button" className="btn ghost" onClick={skip} data-log-label="Einführung überspringen">
            {t("tutorial.skip")}
          </button>
        )}
        {next && (
          <button type="button" className="btn primary" onClick={next} data-log-label="Einführung weiter">
            {t("tutorial.next")}
            <Icon name="next" small />
          </button>
        )}
        {children}
      </span>
    </>
  );
}

/** The step's heading under its sign (an icon of what the step is about). */
function Title({ icon, children }: { icon: IconName; children: ReactNode }) {
  return (
    <>
      <span className="tu-glyph" aria-hidden="true">
        <Icon name={icon} />
      </span>
      <h2 className="tu-title" id="tu-title" tabIndex={-1}>
        {children}
      </h2>
    </>
  );
}

// ---------- The welcome ----------

function Welcome() {
  const [saving, setSaving] = useState<LangId | null>(null);
  const [problem, setProblem] = useState("");
  const current = saving ?? lang();
  const pick = async (id: LangId) => {
    if (id === lang() || saving) return;
    logEvent("info", "settings", "Sprache geändert", { data: { language: id, from: "tutorial" } });
    setSaving(id);
    setProblem("");
    try {
      await postJson("/settings", { language: id }, "PUT");
      focusLanguage = id;
      setLanguage(id); // draws the app anew; the tour stays at this step (tutorial/store.ts)
    } catch (e) {
      setProblem(t("tutorial.failed", { error: errorText(e) }));
      setSaving(null);
    }
  };
  return (
    <div className="tu-welcome">
      <div className="tu-lamp" aria-hidden="true">
        <span className="tu-glow" />
        <BrandMark className="tu-mark" />
      </div>
      <h2 className="tu-word" id="tu-title">
        Lorelamp
      </h2>
      <p className="tu-lead" id="tu-lead">
        {t("tutorial.welcome.lead")}
      </p>
      <fieldset className="tu-langs">
        <legend>{t("tutorial.welcome.language")}</legend>
        {LANGS.map((l) => (
          <label key={l.id} className={`tu-lang${current === l.id ? " on" : ""}`}>
            <input type="radio" name="tu-lang" value={l.id} checked={current === l.id} disabled={saving !== null} onChange={() => void pick(l.id)} data-log-label="Sprache" />
            <span lang={l.id}>{l.name}</span>
          </label>
        ))}
      </fieldset>
      {problem && (
        <p className="bad" role="alert">
          {problem}
        </p>
      )}
      <p className="tu-by">
        <span>{t("tutorial.welcome.by", { name: "Lampe" })}</span>
        <a href={REPO} target="_blank" rel="noopener noreferrer" className="tu-repo">
          <Icon name="link" small />
          {t("tutorial.welcome.code")}
        </a>
      </p>
    </div>
  );
}

// ---------- 1: the model ----------

/** Whether a model is set up: one runs, waits unloaded or starts from a profile; null while the app
 *  does not know yet (no answer from /engine so far). */
function modelSetUp(engine: EngineView): boolean | null {
  const view = known(engine);
  if (!view?.info) return null;
  const kind = modelState(view).kind;
  return kind === "ready" || kind === "idle" || (kind === "loading" && (!view.info.managed || view.info.profile !== null));
}

/** The model's state as `/engine` tells it, also while the app's socket still connects (the first
 *  second or two after a start): the tour does not need the socket to know whether a model is set up.
 *  The same reading of the answer as the app's (App.tsx); null before the first answer. */
function known(engine: EngineView): EngineView | null {
  const info = engine.info;
  if (!info) return null;
  if (engine.status !== "connecting") return engine;
  const status = info.unloaded ? "idle" : info.status === "offline" && !info.note ? "loading" : info.status;
  return { ...engine, status };
}

function ModelStep({ engine, done }: { engine: EngineView; done: boolean }) {
  const name = modelName(engine.info?.model);
  const state = modelState(known(engine) ?? engine);
  return (
    <>
      <Title icon="model">{t("tutorial.model.title")}</Title>
      <p className="tu-lead" id="tu-lead">
        {t("tutorial.model.lead")}
      </p>
      {modelSetUp(engine) === null ? (
        <PanelWait />
      ) : done ? (
        <Done>{name ? t("tutorial.model.ready", { name }) : t("tutorial.model.readySome")}</Done>
      ) : onOtherDevice ? (
        <p className="tu-panel">{t("tutorial.model.atPc")}</p>
      ) : (
        <div className="tu-panel">
          <Suspense fallback={<PanelWait bare />}>
            <EngineSetup note={t("tutorial.model.todo")} acts={() => <ModelAct state={state} primary />} />
          </Suspense>
        </div>
      )}
    </>
  );
}

/** The model step's panel while the app does not know yet what it will hold: a line of text and a
 *  button's room, so the step keeps its height when the real content arrives. `bare`: inside the panel. */
function PanelWait({ bare }: { bare?: boolean }) {
  const lines = (
    <>
      <i className="tu-wait-line" />
      <i className="tu-wait-line short" />
      <EngineWait />
    </>
  );
  return bare ? (
    <div className="tu-wait" aria-hidden="true">
      {lines}
    </div>
  ) : (
    <div className="tu-panel tu-wait" aria-busy="true">
      {lines}
    </div>
  );
}

function Done({ children }: { children: ReactNode }) {
  return (
    <p className="tu-done" role="status">
      <span className="tu-done-mark">
        <Icon name="check" small />
      </span>
      <span>{children}</span>
    </p>
  );
}

// ---------- 2: a character ----------

function FigureStep({ library }: { library: Library | null }) {
  const figures = library?.characters ?? [];
  const some = figures.length > 0;
  return (
    <>
      <Title icon="card">{t("tutorial.figure.title")}</Title>
      <p className="tu-lead" id="tu-lead">
        {t("tutorial.figure.lead")}
      </p>
      {some && (
        <div className="tu-there">
          <Done>{t("tutorial.figure.there", { n: figures.length })}</Done>
          <span className="tu-faces" aria-hidden="true">
            {figures.slice(0, 5).map((c) => (
              <Portrait key={c.id} who={c} className="av" />
            ))}
          </span>
        </div>
      )}
      <div className="tu-acts">
        <a className={`btn ${some ? "solid" : "primary"}`} href="#/import" data-log-label="Karte importieren">
          <Icon name="upload" small />
          {t("tutorial.figure.import")}
        </a>
        <a className="btn solid" href="#/chars/new" data-log-label="Neue Figur bauen">
          <Icon name="plus" small />
          {t("tutorial.figure.build")}
        </a>
      </div>
    </>
  );
}

// ---------- 3: who the user is ----------

function PersonaStep({ library, onDone, onProblem }: { library: Library | null; onDone: () => void; onProblem: (text: string) => void }) {
  const id = useId();
  const [name, setName] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // The name a new installation starts with is not the user's: the field starts empty then.
  const stored = library?.user_name ?? "";
  const shown = name ?? (stored === DEFAULT_USER_NAME ? "" : stored);
  const save = async (e: FormEvent) => {
    e.preventDefault();
    const clean = shown.trim();
    if (!library || busy) return;
    if (clean && clean !== library.user_name) {
      setBusy(true);
      try {
        await postJson("/settings", { user_name: clean }, "PUT");
        logEvent("info", "settings", "Name gesetzt (Einführung)", { data: { chars: clean.length } });
      } catch (err) {
        onProblem(t("tutorial.failed", { error: errorText(err) }));
        setBusy(false);
        return;
      }
      setBusy(false);
    }
    onDone();
  };
  return (
    <>
      <Title icon="persona">{t("tutorial.persona.title")}</Title>
      <p className="tu-lead" id="tu-lead">
        {t("tutorial.persona.lead")}
      </p>
      <form id="tu-persona" className="tu-name" onSubmit={(e) => void save(e)}>
        <label htmlFor={id}>{t("tutorial.persona.name")}</label>
        <input id={id} className="input" value={shown} placeholder={t("tutorial.persona.placeholder")} maxLength={40} autoComplete="nickname" disabled={!library || busy} onChange={(e) => setName(e.target.value)} />
      </form>
      {library && (
        <div className="tu-acts">
          <a className="btn sm ghost" href={`#/personas/${library.active_persona_id}`} data-log-label="Persona anlegen">
            <Icon name="persona" small />
            {t("tutorial.persona.more")}
          </a>
        </div>
      )}
    </>
  );
}

// ---------- 4: the first chat ----------

function ChatStep({ library, quiet, onFigure, onStarted, onProblem }: { library: Library | null; quiet: boolean; onFigure: () => void; onStarted: () => void; onProblem: (text: string) => void }) {
  const figures = library?.characters ?? [];
  const [busy, setBusy] = useState(false);
  const start = async (characterId: number) => {
    setBusy(true);
    try {
      const { chat_id } = await postJson<{ chat_id: number }>("/chats", { character_id: characterId, fresh: true });
      onStarted();
      location.hash = `#/chat/${chat_id}`;
    } catch (e) {
      onProblem(t("tutorial.failed", { error: errorText(e) }));
      setBusy(false);
    }
  };
  return (
    <>
      <Title icon="chat">{t("tutorial.chat.title")}</Title>
      <p className="tu-lead" id="tu-lead">
        {figures.length > 0 ? t("tutorial.chat.lead") : t("tutorial.chat.none")}
      </p>
      {figures.length > 0 ? (
        <>
          <ul className="tu-figs">
            {figures.slice(0, 4).map((c) => (
              <li key={c.id} style={hueStyle(c.name)}>
                <button type="button" className="tu-fig" disabled={busy} onClick={() => void start(c.id)} aria-label={t("tutorial.chat.with", { name: c.name })} data-log-label="Erster Chat (Einführung)">
                  <Portrait who={c} className="av" />
                  <b>{c.name}</b>
                  <Icon name="chat" small />
                </button>
              </li>
            ))}
          </ul>
          {quiet && <p className="tu-hint">{t("tutorial.chat.quiet")}</p>}
        </>
      ) : (
        <div className="tu-acts">
          <button type="button" className="btn solid" onClick={onFigure}>
            <Icon name="back" small />
            {t("tutorial.chat.toFigure")}
          </button>
        </div>
      )}
    </>
  );
}
