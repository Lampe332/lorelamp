import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { token } from "./api";
import { useLang } from "./i18n";
import { linkManifest } from "./phone/shell";
import "./style.css";

linkManifest(token);

/** The app in its language. The language is the app's key: a change draws every screen anew, so
 *  each text is read again in the new language (the page itself is not reloaded, and the place the
 *  user is at lives in the address). */
function Root() {
  const lang = useLang();
  return <App key={lang} />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
