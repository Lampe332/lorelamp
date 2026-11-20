// The reading settings as controls, in six groups behind a row that jumps to each: the layout
// (how messages are set and spaced), type, inks (speech, actions, narration, thoughts), the
// bubbles (fill, width, corners, see-through), name, time and picture at a message, and what happens
// while an answer comes in. Used twice: for the whole app (settings tab "Chat") and for one chat
// (its "Aussehen").

import { useId, useRef, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { numberFormat, t } from "../i18n";
import {
  AV_SHAPES,
  AV_SIZE,
  BUBBLE,
  CORNER,
  DEFAULTS,
  EDGE,
  ENTERS,
  FILL,
  fillColor,
  FONTS,
  FROST,
  GAP,
  HUES,
  inkColor,
  inkHue,
  LINE,
  merge,
  PARA,
  SIZE,
  STYLES,
  twoSided,
  TYPINGS,
  WIDTH,
  type Fill,
  type Ink,
  type Look,
  type StyleId,
} from "./look";

type Props = {
  /** What this level sets itself. */
  value: Look;
  /** What an empty field falls back to: for a chat the app's settings, for the app nothing. */
  base: Look;
  scope: "app" | "chat";
  onChange: (next: Look) => void;
};

type Range = { min: number; max: number; step: number };
type NumKey = "size" | "line" | "width" | "para" | "bubble" | "edge" | "gap" | "corner" | "fill" | "frost" | "av_size";
type BoolKey = "justify" | "speech" | "animate" | "timings" | "say_bold" | "act_italic" | "mine" | "avatars" | "tail" | "shadow" | "names" | "times" | "flip";
type InkKey = "say" | "act" | "story" | "think";
type FillKey = "ai_fill" | "me_fill";
type ChoiceKey = "av_shape" | "enter" | "typing";

/** Arrow keys move through a group of radio buttons and choose, as the platform's own do. */
function radioKeys<T extends string>(all: readonly T[], at: T, choose: (next: T) => void) {
  return (e: KeyboardEvent<HTMLElement>) => {
    const d = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const next = all[(Math.max(0, all.indexOf(at)) + d + all.length) % all.length];
    choose(next);
    (e.currentTarget.parentElement?.querySelector(`[data-value="${next}"]`) as HTMLElement | null)?.focus();
  };
}

/** A small drawing of each way to set messages: the character's lines, then yours. */
function StyleSketch({ id }: { id: StyleId }) {
  return (
    <span className={`cv-sketch cv-sketch-${id}`} aria-hidden="true">
      <i className="a" />
      <i className="a s" />
      <i className="u" />
      <i className="a" />
    </span>
  );
}

export function LookFields({ value, base, scope, onChange }: Props) {
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const shown = merge(merge(DEFAULTS, base), value);
  const set = (patch: Look) => onChange({ ...value, ...patch });
  const inherit = scope === "chat" ? t("chat.look.inherit.chat") : t("chat.look.inherit.app");
  const fmt = (n: number) => numberFormat({ maximumFractionDigits: 2 }).format(n);
  const px = (n: number) => `${fmt(n)} px`;
  const pct = (n: number) => `${fmt(n)} %`;
  const spot = (key: string) => (scope === "app" ? `chat-${key}` : undefined);
  /** Gives one field back to the level above; only offered while this level sets it. */
  const reset = (key: keyof Look, what: string): ReactNode =>
    value[key] !== undefined && (
      <button type="button" className="cv-reset" onClick={() => onChange({ ...value, [key]: undefined })} aria-label={`${what}: ${inherit}`}>
        {inherit}
      </button>
    );

  const range = (key: NumKey, label: string, spec: Range, at: number, text: string, hint?: string) => (
    <div className="cv-field" data-spot={spot(key)}>
      <div className="cv-field-head">
        <label htmlFor={`${id}-${key}`}>{label}</label>
        <output htmlFor={`${id}-${key}`} className="num">
          {text}
        </output>
        {reset(key, label)}
      </div>
      <input
        id={`${id}-${key}`}
        className="cv-range"
        type="range"
        min={spec.min}
        max={spec.max}
        step={spec.step}
        value={at}
        aria-valuetext={text}
        onChange={(e) => set({ [key]: Number(e.target.value) })}
      />
      {hint && <p className="help">{hint}</p>}
    </div>
  );

  const toggle = (key: BoolKey, label: string, hint: string) => (
    <div className="cv-switch" data-spot={spot(key)}>
      <label className="switch-row">
        <span className="sw-text">
          <b>{label}</b>
          <span>{hint}</span>
        </span>
        <input type="checkbox" role="switch" className="switch" checked={shown[key] ?? false} onChange={(e) => set({ [key]: e.target.checked })} />
      </label>
      {reset(key, label)}
    </div>
  );

  /** Named colours, eight hues and a slider for any other hue: as inks of the text, or as fills of
   *  the bubbles (`fill`: drawn as a field with a ring, as light as the theme draws its fields). */
  const swatches = <K extends InkKey | FillKey>(key: K, label: string, named: { id: string; name: string }[], hint: string, kind: "ink" | "fill") => {
    const at = (shown[key] ?? DEFAULTS[key]) as string;
    const hue = inkHue(at as Ink);
    const options = [...named, ...HUES.map((h) => ({ id: `h${h.hue}`, name: h.name }))];
    const ids = options.map((o) => o.id);
    // A hue from the slider that is none of the swatches: no swatch is the chosen one, the slider says it.
    const custom = hue !== null && !ids.includes(at);
    // A fill swatch is the field itself with a ring in the hue's ink: on a dark theme the fields are
    // deep, and the ring tells them apart.
    const swatch = (o: string): CSSProperties =>
      kind === "ink"
        ? { color: inkColor(o as Ink) }
        : ({ color: o === "plain" || o === "none" ? "var(--control)" : inkColor(o as Ink), "--sw-fill": fillColor(o as Fill) } as CSSProperties);
    const choose = (next: string) => set({ [key]: next } as Look);
    return (
      <div className="cv-field" data-spot={spot(key)}>
        <div className="cv-field-head">
          <span id={`${id}-${key}`}>{label}</span>
          <output className="num">{custom ? t("chat.look.hue", { hue: hue ?? 0 }) : options.find((o) => o.id === at)?.name}</output>
          {reset(key, label)}
        </div>
        <div className="cv-inks" role="radiogroup" aria-labelledby={`${id}-${key}`}>
          {options.map((o, i) => (
            <button
              key={o.id}
              type="button"
              role="radio"
              aria-checked={at === o.id}
              aria-label={o.name}
              title={o.name}
              className={`cv-ink${kind === "fill" ? " cv-fill" : ""}`}
              data-value={o.id}
              tabIndex={at === o.id || (custom && i === 0) ? 0 : -1}
              style={swatch(o.id)}
              onClick={() => choose(o.id)}
              onKeyDown={radioKeys(ids, at, choose)}
            >
              <i />
            </button>
          ))}
        </div>
        <div className="cv-hue">
          <label htmlFor={`${id}-${key}-hue`}>{t("chat.look.ownHue")}</label>
          <input
            id={`${id}-${key}-hue`}
            className="cv-range cv-range-hue"
            type="range"
            min={0}
            max={359}
            step={1}
            value={hue ?? 30}
            aria-valuetext={hue === null ? t("chat.look.hue.none") : t("chat.look.hue.deg", { hue })}
            onChange={(e) => choose(`h${Number(e.target.value)}`)}
          />
        </div>
        <p className="help">{hint}</p>
      </div>
    );
  };

  /** A few named choices side by side. */
  const choice = <K extends ChoiceKey>(key: K, label: string, options: { id: NonNullable<Look[K]>; name: string }[], hint?: string) => {
    const at = (shown[key] ?? DEFAULTS[key]) as NonNullable<Look[K]>;
    const ids = options.map((o) => o.id);
    const choose = (next: NonNullable<Look[K]>) => set({ [key]: next } as Look);
    return (
      <div className="cv-field" data-spot={spot(key)}>
        <div className="cv-field-head">
          <span id={`${id}-${key}`}>{label}</span>
          {reset(key, label)}
        </div>
        <div className="seg cv-seg cv-choice" role="radiogroup" aria-labelledby={`${id}-${key}`}>
          {options.map((o) => (
            <button
              key={o.id}
              type="button"
              role="radio"
              aria-checked={at === o.id}
              data-value={o.id}
              tabIndex={at === o.id ? 0 : -1}
              onClick={() => choose(o.id)}
              onKeyDown={radioKeys(ids, at, choose)}
            >
              {o.name}
            </button>
          ))}
        </div>
        {hint && <p className="help">{hint}</p>}
      </div>
    );
  };

  const styleIds = STYLES.map((s) => s.id);
  const style = shown.style ?? DEFAULTS.style;
  const speechOn = shown.speech ?? DEFAULTS.speech;
  const avatarsOn = (shown.avatars ?? DEFAULTS.avatars) && style !== "compact";
  const inkFigure = { id: "figure", name: t("chat.look.ink.figure") };
  const inkText = { id: "text", name: t("chat.look.ink.text") };
  const inkSoft = { id: "soft", name: t("chat.look.ink.soft") };
  const fillsNamed = (first: "figure" | "plain") => {
    const figure = { id: "figure", name: t("chat.look.fill.figure") };
    const plain = { id: "plain", name: t("chat.look.fill.plain") };
    return [...(first === "figure" ? [figure, plain] : [plain, figure]), { id: "none", name: t("chat.look.fill.none") }];
  };

  const groups: { key: string; title: string }[] = [
    { key: "layout", title: t("chat.look.g.style") },
    { key: "type", title: t("chat.look.g.type") },
    { key: "ink", title: t("chat.look.g.ink") },
    { key: "bubbles", title: t("chat.look.g.bubbles") },
    { key: "head", title: t("chat.look.g.head") },
    { key: "live", title: t("chat.look.g.live") },
  ];
  const jump = (key: string) => root.current?.querySelector<HTMLElement>(`[data-group="${key}"]`)?.scrollIntoView({ block: "start", behavior: "smooth" });
  const group = (key: string, children: ReactNode) => {
    const g = groups.find((x) => x.key === key)!;
    return (
      <section className="cv-group" data-group={key} aria-labelledby={`${id}-g-${key}`}>
        <h3 id={`${id}-g-${key}`}>{g.title}</h3>
        {children}
      </section>
    );
  };

  return (
    <div className="cv-fields" ref={root}>
      <nav className="cv-jump" aria-label={t("chat.look.jump")}>
        {groups.map((g) => (
          <button key={g.key} type="button" className="cv-jump-btn" onClick={() => jump(g.key)}>
            {g.title}
          </button>
        ))}
      </nav>

      {group(
        "layout",
        <>
          <div className="cv-field" data-spot={spot("style")}>
            <div className="cv-field-head">
              <span id={`${id}-style`}>{t("chat.look.style")}</span>
              {reset("style", t("chat.look.style"))}
            </div>
            <div className="cv-styles" role="radiogroup" aria-labelledby={`${id}-style`}>
              {STYLES.map((s) => (
                <button
                  key={s.id}
                  type="button"
                  role="radio"
                  aria-checked={style === s.id}
                  className="cv-style"
                  data-value={s.id}
                  tabIndex={style === s.id ? 0 : -1}
                  onClick={() => set({ style: s.id })}
                  onKeyDown={radioKeys(styleIds, style, (next) => set({ style: next }))}
                >
                  <StyleSketch id={s.id} />
                  <span>{s.name}</span>
                </button>
              ))}
            </div>
            <p className="help">{STYLES.find((s) => s.id === style)?.note}</p>
          </div>
          {twoSided(style) && toggle("flip", t("chat.look.flip"), t("chat.look.flip.note"))}
          {style === "wide"
            ? range("edge", t("chat.look.edge"), EDGE, shown.edge ?? 32, shown.edge !== undefined ? px(shown.edge) : t("chat.look.auto"), shown.edge !== undefined ? undefined : t("chat.look.edge.auto"))
            : range("width", t("chat.look.width"), WIDTH, shown.width ?? 700, shown.width ? px(shown.width) : t("chat.look.auto"), shown.width ? undefined : t("chat.look.width.auto"))}
          {range("gap", t("chat.look.gap"), GAP, shown.gap ?? 30, shown.gap !== undefined ? px(shown.gap) : t("chat.look.auto"), shown.gap !== undefined ? undefined : t("chat.look.gap.auto"))}
        </>,
      )}

      {group(
        "type",
        <>
          <div className="cv-field" data-spot={spot("font")}>
            <div className="cv-field-head">
              <span id={`${id}-font`}>{t("chat.look.font")}</span>
              {reset("font", t("chat.look.font"))}
            </div>
            <div className="cv-fonts" role="radiogroup" aria-labelledby={`${id}-font`}>
              {FONTS.map((f) => (
                <button
                  key={f.id}
                  type="button"
                  role="radio"
                  aria-checked={shown.font === f.id}
                  className="cv-font"
                  onClick={() => set({ font: f.id })}
                  onKeyDown={radioKeys(
                    FONTS.map((x) => x.id),
                    f.id,
                    (next) => set({ font: next }),
                  )}
                  tabIndex={shown.font === f.id ? 0 : -1}
                  data-font={f.id}
                  data-value={f.id}
                >
                  <span className="cv-font-aa" style={f.stack ? { fontFamily: f.stack } : undefined} aria-hidden="true">
                    Aa
                  </span>
                  <span className="cv-font-name">
                    {f.name}
                    <small>{f.note}</small>
                  </span>
                </button>
              ))}
            </div>
            <p className="help">{t("chat.look.font.help")}</p>
          </div>
          {range("size", t("chat.look.size"), SIZE, shown.size ?? 19, shown.size ? px(shown.size) : t("chat.look.auto"), shown.size ? undefined : t("chat.look.size.auto"))}
          {range("line", t("chat.look.line"), LINE, shown.line ?? DEFAULTS.line, fmt(shown.line ?? DEFAULTS.line))}
          {range("para", t("chat.look.para"), PARA, shown.para ?? DEFAULTS.para, t("chat.look.para.value", { n: Math.round((shown.para ?? DEFAULTS.para) * 100) / 100 }))}
          {toggle("justify", t("chat.look.justify"), t("chat.look.justify.note"))}
        </>,
      )}

      {group(
        "ink",
        <>
          {toggle("speech", t("chat.look.speech"), t("chat.look.speech.note"))}
          {speechOn && swatches("say", t("chat.look.say"), [inkFigure, inkText], t("chat.look.say.note"), "ink")}
          {speechOn && toggle("say_bold", t("chat.look.sayBold"), t("chat.look.sayBold.note"))}
          {swatches("act", t("chat.look.act"), [inkSoft, inkText, inkFigure], t("chat.look.act.note"), "ink")}
          {toggle("act_italic", t("chat.look.actItalic"), t("chat.look.actItalic.note"))}
          {swatches("story", t("chat.look.story"), [inkText, inkSoft, inkFigure], t("chat.look.story.note"), "ink")}
          {swatches("think", t("chat.look.think"), [inkSoft, inkText, inkFigure], t("chat.look.think.note"), "ink")}
          {toggle("mine", t("chat.look.mine"), t("chat.look.mine.note"))}
        </>,
      )}

      {group(
        "bubbles",
        <>
          {swatches("ai_fill", t("chat.look.aiFill"), fillsNamed("figure"), style === "book" || style === "compact" ? t("chat.look.aiFill.book") : t("chat.look.aiFill.note"), "fill")}
          {swatches("me_fill", t("chat.look.meFill"), fillsNamed("plain"), t("chat.look.meFill.note"), "fill")}
          {range("bubble", t("chat.look.bubble"), BUBBLE, shown.bubble ?? 80, shown.bubble !== undefined ? pct(shown.bubble) : t("chat.look.auto"), shown.bubble !== undefined ? undefined : t("chat.look.bubble.auto"))}
          {range("corner", t("chat.look.corner"), CORNER, shown.corner ?? 18, shown.corner !== undefined ? px(shown.corner) : t("chat.look.auto"))}
          {toggle("tail", t("chat.look.tail"), t("chat.look.tail.note"))}
          {range("fill", t("chat.look.cover"), FILL, shown.fill ?? DEFAULTS.fill, pct(shown.fill ?? DEFAULTS.fill), t("chat.look.cover.note"))}
          {range("frost", t("chat.look.frost"), FROST, shown.frost ?? DEFAULTS.frost, (shown.frost ?? 0) > 0 ? px(shown.frost ?? 0) : t("chat.look.off"), t("chat.look.frost.note"))}
          {toggle("shadow", t("chat.look.shadow"), t("chat.look.shadow.note"))}
        </>,
      )}

      {group(
        "head",
        <>
          {toggle("names", t("chat.look.names"), t("chat.look.names.note"))}
          {toggle("times", t("chat.look.times"), t("chat.look.times.note"))}
          {toggle("avatars", t("chat.look.avatars"), t("chat.look.avatars.note"))}
          {avatarsOn && range("av_size", t("chat.look.avSize"), AV_SIZE, shown.av_size ?? 28, shown.av_size !== undefined ? px(shown.av_size) : t("chat.look.auto"))}
          {avatarsOn && choice("av_shape", t("chat.look.avShape"), AV_SHAPES)}
        </>,
      )}

      {group(
        "live",
        <>
          {choice("enter", t("chat.look.enter"), ENTERS, t("chat.look.enter.note"))}
          {choice("typing", t("chat.look.typing"), TYPINGS, t("chat.look.typing.note"))}
          {toggle("animate", t("chat.look.animate"), t("chat.look.animate.note"))}
          {toggle("timings", t("chat.look.timings"), t("chat.look.timings.note"))}
        </>,
      )}
    </div>
  );
}
