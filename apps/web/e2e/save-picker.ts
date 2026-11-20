// The save dialog (`window.showSaveFilePicker`) in tests. Headless Edge cancels it at once, so a test
// either takes it away (the page falls back to a plain download, Playwright's `download` event) or
// replaces it with a recorder that keeps what the page writes.

import type { Page } from "@playwright/test";

/** Without the save dialog: exports go through the browser's download. */
export async function withoutSavePicker(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    delete w.showSaveFilePicker;
    if ("showSaveFilePicker" in w) Object.defineProperty(window, "showSaveFilePicker", { value: undefined });
  });
}

/** One file the page saved through the recorded dialog. */
export type Saved = {
  suggestedName: string;
  types: { description: string; accept: Record<string, string[]> }[] | undefined;
  /** The click that opened the dialog was still fresh (a real dialog needs that). */
  activated: boolean;
  /** Export requests the page had made before it opened the dialog. */
  fetchedBefore: number;
  closed: boolean;
  base64: string;
};

/** Replaces the save dialog by a recorder. `setSaveAnswer(page, "cancel")` makes it act like the user
 *  closed the dialog without saving. */
export async function recordSavePicker(page: Page) {
  await page.addInitScript(() => {
    type Rec = { suggestedName: string; types: unknown; activated: boolean; fetchedBefore: number; closed: boolean; bytes: number[] };
    const w = window as unknown as { __saves: Rec[]; __saveAnswer: string; showSaveFilePicker: unknown };
    w.__saves = [];
    w.__saveAnswer = "save";
    w.showSaveFilePicker = async (opts: { suggestedName: string; types?: unknown }) => {
      if (w.__saveAnswer === "cancel") throw new DOMException("The user aborted a request.", "AbortError");
      const rec: Rec = {
        suggestedName: opts.suggestedName,
        types: opts.types,
        activated: navigator.userActivation.isActive,
        fetchedBefore: performance.getEntriesByType("resource").filter((e) => /\/export/.test(e.name)).length,
        closed: false,
        bytes: [],
      };
      w.__saves.push(rec);
      return {
        name: opts.suggestedName,
        createWritable: async () => ({
          write: async (data: Blob | ArrayBuffer) => {
            const buf = data instanceof Blob ? await data.arrayBuffer() : data;
            rec.bytes.push(...new Uint8Array(buf));
          },
          close: async () => {
            rec.closed = true;
          },
        }),
      };
    };
  });
}

export const setSaveAnswer = (page: Page, answer: "save" | "cancel") =>
  page.evaluate((a) => ((window as unknown as { __saveAnswer: string }).__saveAnswer = a), answer);

/** Everything saved so far, the bytes as base64. */
export async function saves(page: Page): Promise<Saved[]> {
  return page.evaluate(() => {
    const w = window as unknown as { __saves: { suggestedName: string; types: never; activated: boolean; fetchedBefore: number; closed: boolean; bytes: number[] }[] };
    return w.__saves.map(({ bytes, ...rest }) => {
      let bin = "";
      for (const b of bytes) bin += String.fromCharCode(b);
      return { ...rest, base64: btoa(bin) };
    });
  });
}
