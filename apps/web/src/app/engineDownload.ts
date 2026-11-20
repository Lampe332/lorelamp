// The engine download's socket events ({"type":"engine_download",…}), passed on by the app's one
// socket (useChatSocket) to whoever shows the download (the settings tab "Modell").

import { useEffect } from "react";

export type SelfTest = {
  ok: boolean;
  /** What was run: "list-devices", "version"; empty for a Vulkan build (never started for a test). */
  ran: string[];
  /** GPUs the build listed ("ROCm0: AMD Radeon RX 7900 XTX (24560 MiB, …)"). */
  devices: string[];
  version: string | null;
  at: number;
};

export type EngineDownloadEvent = {
  type: "engine_download";
  tag: string;
  asset: string;
  variant: string;
  backend: string;
  phase: "download" | "unpack" | "test" | "done" | "failed";
  downloaded: number;
  total: number;
  engine_id?: number;
  selftest?: SelfTest;
  /** Why it failed (`hash_mismatch`, `rocm_no_gpu`, …). */
  error?: string;
};

const NAME = "rp-engine-download";

export const forwardEngineDownload = (ev: EngineDownloadEvent) => window.dispatchEvent(new CustomEvent<EngineDownloadEvent>(NAME, { detail: ev }));

/** Calls `fn` with each download event while the component is mounted. */
export function useEngineDownload(fn: (ev: EngineDownloadEvent) => void) {
  useEffect(() => {
    const on = (e: Event) => fn((e as CustomEvent<EngineDownloadEvent>).detail);
    window.addEventListener(NAME, on);
    return () => window.removeEventListener(NAME, on);
  }, [fn]);
}
