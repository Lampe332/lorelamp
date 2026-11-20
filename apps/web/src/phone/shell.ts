// Where this page runs: on the PC that runs Lorelamp, or on another device (a phone in the Wi-Fi,
// the Android app). Contract with the Android app: it opens `http://<ip>:<port>/?token=…&shell=android`
// and offers its own start page again at `http://tauri.localhost/?rescan=1` ("Anderen PC verbinden").

import { apiUrl } from "../api";

const SHELL_KEY = "rp-shell";

/** `android` inside the Android app; kept for the tab, so a reload without the parameter stays in it. */
const shell = ((): string | null => {
  const fromUrl = new URLSearchParams(location.search).get("shell");
  try {
    if (fromUrl) sessionStorage.setItem(SHELL_KEY, fromUrl);
    return fromUrl ?? sessionStorage.getItem(SHELL_KEY);
  } catch {
    return fromUrl;
  }
})();

const LOOPBACK = /^(localhost|127(?:\.\d{1,3}){3}|\[::1\]|::1)$/i;

/** The page runs in the Android app. */
export const inApp = shell === "android";

/** The server this page talks to runs on another device than the page: a phone, a tablet, another
 *  PC. Its engines, model folders, benchmark and backups belong to that PC. */
export const onOtherDevice = inApp || !LOOPBACK.test(new URL(apiUrl(""), location.href).hostname);

/** The Android app's own start page, where it searches the Wi-Fi again. */
export const RESCAN_URL = "http://tauri.localhost/?rescan=1";

/** "Zum Startbildschirm": the manifest's start address then carries this page's token (the server
 *  checks it), so Lorelamp opened from the home screen is connected at once. Only for the server that
 *  delivered this page. */
export function linkManifest(token: string) {
  const link = document.querySelector<HTMLLinkElement>('link[rel="manifest"]');
  if (link && token && apiUrl("").startsWith("/")) link.href = `/manifest.webmanifest?token=${encodeURIComponent(token)}`;
}
