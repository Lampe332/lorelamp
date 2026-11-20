// Settings tab "Chat": how every chat reads, in six groups (layout, bubbles, type, inks, name/time/
// picture, what happens while an answer comes in). A sample beside the
// controls follows each change at once; one chat can still differ through its own "Aussehen".

import { useEffect, useRef, useState } from "react";
import { LookFields } from "../chat/LookFields";
import { lookProps, messageTime, resolve, useGlobalLook, useReducedMotion } from "../chat/look";
import { Prose } from "../chat/Prose";
import "../chat/chat.css";
import { t } from "../i18n";
import { respot } from "../spot";
import { hueStyle } from "../ui";

/** The sample figure's colour comes from this name in every language. */
const SAMPLE_NAME = "Mara";

export function ChatTab() {
  // The sample exchange is the app's own text, so it is read in the language on screen.
  const sampleAi = t("chat.sample.ai");
  const sampleName = t("chat.sample.name");
  const { look, failed, change } = useGlobalLook();
  const reduced = useReducedMotion();
  // The sample answer written out again, the way a real one arrives: a few letters per step.
  const [typed, setTyped] = useState<number | null>(null);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearInterval(timer.current), []);
  const loaded = look !== null;
  useEffect(() => {
    if (loaded) respot();
  }, [loaded]);

  if (!look) {
    return (
      <div className="view">
        <div className="page set" aria-busy={!failed}>
          {failed ? <p className="bad">{t("chat.tab.failed")}</p> : <span className="bar-ph" />}
        </div>
      </div>
    );
  }

  const shown = resolve(look);
  const attrs = lookProps(shown);
  const animate = shown.animate !== false && !reduced;
  const play = () => {
    window.clearInterval(timer.current);
    setTyped(0);
    timer.current = window.setInterval(() => {
      setTyped((n) => {
        const next = (n ?? 0) + 3;
        if (next < sampleAi.length) return next;
        window.clearInterval(timer.current);
        return null;
      });
    }, 40);
  };
  const changed = Object.keys(look).length > 0;
  /** The sample's times: a few minutes ago, as a chat shows them when "Uhrzeit zeigen" is on. */
  const time = (minutesAgo: number) => {
    if (!shown.times) return null;
    const at = Math.floor(Date.now() / 1000) - minutesAgo * 60;
    return (
      <time className="msg-time num" dateTime={new Date(at * 1000).toISOString()}>
        {messageTime(at)}
      </time>
    );
  };

  return (
    <div className="view">
      <div className="page set">
        <div className="cv-tab">
          <div className="set-panel" aria-label={t("chat.tab.label")}>
            <LookFields value={look} base={{}} scope="app" onChange={change} />
            <div className="btn-row">
              <button type="button" className="btn ghost" disabled={!changed} onClick={() => change({})}>
                {t("chat.tab.reset")}
              </button>
            </div>
            <p className="help">{t("chat.tab.help")}</p>
          </div>

          <section className="cv-sample" aria-label={t("chat.tab.preview")}>
            <div className="cv-sample-head">
              <h2>{t("chat.tab.preview")}</h2>
              <button type="button" className="btn sm" onClick={play} disabled={typed !== null}>
                {t("chat.tab.play")}
              </button>
            </div>
            <div className="cv-sample-box" style={{ ...hueStyle(SAMPLE_NAME), ...attrs.style }} {...attrs.data}>
              <div className="col">
                <article className="msg ai" aria-label={t("chat.tab.sampleAi")}>
                  <header className="msg-head">
                    <span className="av initial" style={hueStyle(SAMPLE_NAME)} aria-hidden="true">
                      {sampleName[0]}
                    </span>
                    <b>
                      <span className="msg-name">{sampleName}</span>
                      {time(3)}
                    </b>
                  </header>
                  <Prose text={typed === null ? sampleAi : sampleAi.slice(0, typed)} streaming={typed !== null} animate={animate} who={sampleName} />
                </article>
                <article className="msg user" aria-label={t("chat.tab.sampleMine")}>
                  <header className="msg-head">
                    <b>
                      <span className="msg-name">{t("chat.msg.you")}</span>
                      {time(2)}
                    </b>
                  </header>
                  <Prose text={t("chat.sample.user")} streaming={false} />
                </article>
                <article className="msg ai" aria-label={t("chat.tab.sampleAi2")}>
                  <header className="msg-head">
                    <span className="av initial" style={hueStyle(SAMPLE_NAME)} aria-hidden="true">
                      {sampleName[0]}
                    </span>
                    <b>
                      <span className="msg-name">{sampleName}</span>
                      {time(1)}
                    </b>
                  </header>
                  <Prose text={t("chat.sample.ai2")} streaming={false} />
                </article>
              </div>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
