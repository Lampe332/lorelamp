// Layouts of the characters tab. Kept like the start layout: on the server ("library_layout"), with a
// local copy so the tab does not jump from one layout to another while the library loads.

import { texts } from "../i18n";

export const LIBRARY_LAYOUTS = [
  texts({ id: "poster" }, { name: "shell.libraryLayout.poster", note: "shell.libraryLayout.poster.note" }),
  texts({ id: "compact" }, { name: "shell.libraryLayout.compact", note: "shell.libraryLayout.compact.note" }),
  texts({ id: "list" }, { name: "shell.libraryLayout.list", note: "shell.libraryLayout.list.note" }),
] as const;

export type LibraryLayoutId = (typeof LIBRARY_LAYOUTS)[number]["id"];
export const DEFAULT_LIBRARY_LAYOUT: LibraryLayoutId = "poster";
const KEY = "rp-library-layout";

export const isLibraryLayout = (v: unknown): v is LibraryLayoutId => LIBRARY_LAYOUTS.some((l) => l.id === v);

/** The layout this device used last, until the server's arrives. */
export function storedLibraryLayout(): LibraryLayoutId | null {
  try {
    const v = localStorage.getItem(KEY);
    return isLibraryLayout(v) ? v : null;
  } catch {
    return null;
  }
}

export function rememberLibraryLayout(id: LibraryLayoutId) {
  try {
    localStorage.setItem(KEY, id);
  } catch {
    // Private windows and blocked storage: the server still has the choice.
  }
}

// A layout chosen in the quick actions: the characters tab takes it (and saves it) once it is on screen.
let requested: LibraryLayoutId | null = null;
export const LIBRARY_LAYOUT_EVENT = "rp-library-layout";

/** Opens the characters tab in this layout. */
export function requestLibraryLayout(id: LibraryLayoutId) {
  requested = id;
  rememberLibraryLayout(id);
  if (location.hash !== "#/chars") location.hash = "#/chars";
  window.dispatchEvent(new Event(LIBRARY_LAYOUT_EVENT));
}

/** The layout asked for in the quick actions, once. */
export function takeRequestedLayout(): LibraryLayoutId | null {
  const r = requested;
  requested = null;
  return r;
}
