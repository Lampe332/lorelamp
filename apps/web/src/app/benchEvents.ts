// The benchmark's socket events ({"type":"bench",…}), passed on by the app's one socket
// (useChatSocket) to whoever shows the benchmark (the model settings).

import { useEffect } from "react";
import type { BenchEvent } from "../api";

const NAME = "rp-bench";

export const forwardBench = (ev: BenchEvent) => window.dispatchEvent(new CustomEvent<BenchEvent>(NAME, { detail: ev }));

/** Calls `fn` with each benchmark event while the component is mounted. */
export function useBenchEvents(fn: (ev: BenchEvent) => void) {
  useEffect(() => {
    const on = (e: Event) => fn((e as CustomEvent<BenchEvent>).detail);
    window.addEventListener(NAME, on);
    return () => window.removeEventListener(NAME, on);
  }, [fn]);
}
