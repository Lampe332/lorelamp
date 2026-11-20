// Mounted once by App.tsx: asks the server where the welcome tour stands and, only while it is on,
// loads the tour (Tour.tsx, its own chunk). An installation that has finished it never loads more
// than this file.

import { lazy, Suspense, useEffect } from "react";
import type { Library } from "../api";
import type { EngineView } from "../hub/blocks";
import { loadTour, useTour } from "./store";

const Tour = lazy(() => import("./Tour").then((m) => ({ default: m.Tour })));

export function TutorialHost({ library, engine }: { library: Library | null; engine: EngineView }) {
  const { step, docked } = useTour();
  useEffect(() => void loadTour(), []);
  if (step === null || step === "done") return null;
  return (
    <Suspense fallback={null}>
      <Tour step={step} docked={docked} library={library} engine={engine} />
    </Suspense>
  );
}
