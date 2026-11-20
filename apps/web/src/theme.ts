// Colour themes. The colours live in style.css ([data-theme] blocks); this file names them and puts
// the chosen one on <html>. The server keeps the choice (setting "theme", same for every device);
// localStorage keeps a copy so index.html can set it before the first paint.

import { texts } from "./i18n";

export const THEMES = [
  texts({ id: "graphit" }, { name: "shell.theme.graphit" }),
  texts({ id: "glut" }, { name: "shell.theme.glut" }),
  texts({ id: "mitternacht" }, { name: "shell.theme.mitternacht" }),
  texts({ id: "smaragd" }, { name: "shell.theme.smaragd" }),
  texts({ id: "bordeaux" }, { name: "shell.theme.bordeaux" }),
  texts({ id: "papier" }, { name: "shell.theme.papier" }),
] as const;

export type ThemeId = (typeof THEMES)[number]["id"];
export const DEFAULT_THEME: ThemeId = "graphit";
export const STORAGE_KEY = "rp-theme";

export const isTheme = (v: unknown): v is ThemeId => THEMES.some((t) => t.id === v);

/** Layouts of the start screen. Kept like the theme: on the server ("home_layout"), with a local
 *  copy so the start screen does not jump from one layout to another while the library loads. */
export const HOME_LAYOUTS = [
  texts({ id: "bisher" }, { name: "shell.layout.bisher", note: "shell.layout.bisher.note" }),
  texts({ id: "dashboard" }, { name: "shell.layout.dashboard", note: "shell.layout.dashboard.note" }),
  texts({ id: "bento" }, { name: "shell.layout.bento", note: "shell.layout.bento.note" }),
  texts({ id: "buehne" }, { name: "shell.layout.buehne", note: "shell.layout.buehne.note" }),
  texts({ id: "kommando" }, { name: "shell.layout.kommando", note: "shell.layout.kommando.note" }),
] as const;

export type HomeLayoutId = (typeof HOME_LAYOUTS)[number]["id"];
export const DEFAULT_HOME_LAYOUT: HomeLayoutId = "bisher";
const LAYOUT_KEY = "rp-home-layout";

export const isHomeLayout = (v: unknown): v is HomeLayoutId => HOME_LAYOUTS.some((l) => l.id === v);

/** The layout this device used last, until the server's arrives. */
export function storedHomeLayout(): HomeLayoutId | null {
  try {
    const v = localStorage.getItem(LAYOUT_KEY);
    return isHomeLayout(v) ? v : null;
  } catch {
    return null;
  }
}

export function rememberHomeLayout(id: HomeLayoutId) {
  try {
    localStorage.setItem(LAYOUT_KEY, id);
  } catch {
    // Private windows and blocked storage: the server still has the choice.
  }
}

/** Puts a theme on the page: the attribute the CSS reads, the browser's light/dark hint and the
 *  colour of the phone's browser bar, plus the local copy for the next start. */
export function applyTheme(id: ThemeId) {
  const root = document.documentElement;
  if (root.dataset.theme !== id) root.dataset.theme = id;
  const styles = getComputedStyle(root);
  const scheme = styles.colorScheme || "dark";
  document.querySelector('meta[name="color-scheme"]')?.setAttribute("content", scheme);
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", styles.getPropertyValue("--ground").trim());
  try {
    localStorage.setItem(STORAGE_KEY, id);
  } catch {
    // Private windows and blocked storage: the server still has the choice.
  }
}
