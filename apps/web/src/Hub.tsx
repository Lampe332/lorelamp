// The start screen. Its one question: where do I carry on?
// Five layouts to choose from (setting "Start-Layout", also switchable on the screen itself): the one
// before ("bisher") and four more. Every figure on them comes from the library; what is planned but
// not built is named in one line at the foot.
// The layouts live in ./hub, one file each, with the pieces they share in ./hub/blocks (the last chat,
// rows, tiles, measurements) and ./hub/extras (model state with its action, first steps, new chat and
// star on a figure, new imports, the layout switch). Each layout is a chunk of its own: a device only
// loads the one it shows (the others follow when the browser is idle, see `loadLayouts`).

import { lazy, Suspense, type ReactNode } from "react";
import { derive, type HubProps } from "./hub/blocks";
import { HubToolsProvider, ModelCall } from "./hub/extras";
import { DEFAULT_HOME_LAYOUT, storedHomeLayout, type HomeLayoutId } from "./theme";
import { t } from "./i18n";

export type { EngineView } from "./hub/blocks";

const LOAD = {
  bisher: () => import("./hub/Classic").then((m) => ({ default: m.Classic })),
  dashboard: () => import("./hub/Dashboard").then((m) => ({ default: m.Dashboard })),
  bento: () => import("./hub/Bento").then((m) => ({ default: m.Bento })),
  buehne: () => import("./hub/Stage").then((m) => ({ default: m.Stage })),
  kommando: () => import("./hub/Command").then((m) => ({ default: m.CommandCenter })),
};
void (LOAD satisfies Record<HomeLayoutId, unknown>); // one loader per layout id

const Classic = lazy(LOAD.bisher);
const Dashboard = lazy(LOAD.dashboard);
const Bento = lazy(LOAD.bento);
const Stage = lazy(LOAD.buehne);
const CommandCenter = lazy(LOAD.kommando);

// The layout this device showed last is fetched at once, beside the library.
void LOAD[storedHomeLayout() ?? DEFAULT_HOME_LAYOUT]().catch(() => {});

/** Fetches every layout (when the browser is idle), so switching never waits. */
export function loadLayouts(): Promise<unknown> {
  return Promise.all(Object.values(LOAD).map((load) => load()));
}

/** The start screen before its data (or its layout) is there. */
function Loading({ notes, busy }: { notes: ReactNode; busy: boolean }) {
  return (
    <div className="view">
      <div className="page hub">
        <div className="title-row">
          <h1>{t("shell.nav.start")}</h1>
        </div>
        {notes}
        {busy && (
          <div className="resume placeholder" aria-busy="true">
            <span className="pic" />
            <div>
              <span className="bar-ph short" />
              <span className="bar-ph" />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export function Hub(props: HubProps) {
  const { library, loadError, engineNote, onRetry, layout, engine } = props;
  const notes = (
    <>
      {/* Before the first chat the first steps say it: the model is their step one. */}
      {!(library && library.recent.length === 0) && <ModelCall engine={engine} note={engineNote} />}
      {loadError && (
        <div className="well result" role="alert">
          <p>{t("system.hub.loadFailed", { error: loadError })}</p>
          <button className="btn sm" onClick={onRetry}>
            {t("system.retry")}
          </button>
        </div>
      )}
    </>
  );

  if (!library) return <Loading notes={notes} busy={!loadError} />;

  const data = derive(library, props.live);
  const view = { ...props, data, notes };
  return (
    <HubToolsProvider value={{ onNewChat: props.onNewChat, onImport: props.onImport, commands: props.commands }}>
      <Suspense fallback={<Loading notes={notes} busy />}>
        {layout === "dashboard" ? (
          <Dashboard {...view} />
        ) : layout === "bento" ? (
          <Bento {...view} />
        ) : layout === "buehne" ? (
          <Stage {...view} />
        ) : layout === "kommando" ? (
          <CommandCenter {...view} />
        ) : (
          <Classic {...view} />
        )}
      </Suspense>
    </HubToolsProvider>
  );
}
