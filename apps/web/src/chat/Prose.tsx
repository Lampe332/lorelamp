// The text of a message: paragraphs, *actions* and **strong** words, spoken words set apart, the
// words of a search marked, and, while an answer is written, each new piece of text fading in.
// Card and model text is never inserted as HTML.

import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { t } from "../i18n";
import { fold } from "../search";

/** Ranges [start, end) in `text` where one of the folded `words` occurs, sorted and merged. */
function findRanges(text: string, words: string[]): [number, number][] {
  const f = fold(text);
  const out: [number, number][] = [];
  for (const w of words) {
    for (let at = f.text.indexOf(w); at !== -1; at = f.text.indexOf(w, at + w.length)) out.push([f.src[at], f.src[at + w.length - 1] + 1]);
  }
  out.sort((a, b) => a[0] - b[0]);
  const merged: [number, number][] = [];
  for (const r of out) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else merged.push([r[0], r[1]]);
  }
  return merged;
}

/** Plain text with the search words set apart. */
function marked(text: string, words: string[]): ReactNode {
  const ranges = findRanges(text, words);
  if (ranges.length === 0) return text;
  const parts: ReactNode[] = [];
  let at = 0;
  ranges.forEach(([a, b], i) => {
    if (a > at) parts.push(text.slice(at, a));
    parts.push(<mark key={i}>{text.slice(a, b)}</mark>);
    at = b;
  });
  if (at < text.length) parts.push(text.slice(at));
  return parts;
}

/** How long a new piece of text takes to appear (ms); matches `.cv-in` in chat.css. */
export const FADE_MS = 420;

/** A piece of streamed text that is still appearing: where it starts in the message, and since when. */
type Fresh = { at: number; born: number };

type Ctx = {
  words: string[] | null;
  /** Pieces still appearing, in order; empty = everything is plain text. */
  fresh: Fresh[];
  now: number;
  /** The text may end inside a sentence being spoken (an answer still being written). */
  open: boolean;
};

/** Starts a piece's fade where its age puts it, once: a piece that moves into another element (its
 *  `*action*` closed) goes on from there instead of starting over. */
const startAt = (age: number) => (el: HTMLSpanElement | null) => {
  if (!el || el.dataset.t) return;
  el.dataset.t = "1";
  el.style.animationDelay = `${-Math.round(age)}ms`;
};

/** The smallest unit: plain text that starts at `at` of the message. */
function leaf(text: string, at: number, ctx: Ctx): ReactNode {
  const { fresh } = ctx;
  if (fresh.length === 0 || at + text.length <= fresh[0].at) return ctx.words ? marked(text, ctx.words) : text;
  const out: ReactNode[] = [];
  const settled = Math.max(0, fresh[0].at - at);
  if (settled > 0) out.push(text.slice(0, settled));
  fresh.forEach((f, i) => {
    const from = Math.max(f.at, at);
    const to = Math.min(fresh[i + 1]?.at ?? Infinity, at + text.length);
    if (to <= from) return;
    out.push(
      <span key={f.at} className="cv-in" ref={startAt(ctx.now - f.born)}>
        {text.slice(from - at, to - at)}
      </span>,
    );
  });
  return out;
}

// Spoken words: "…", „…“, “…”, «…», »…«. A closing mark of another style ends them too.
const SPEECH = /"[^"\n]+"|„[^“”"\n]+[“”"]|“[^”"\n]+[”"]|«[^»\n]+»|»[^«\n]+«/g;
/** An opening mark with no end yet, at the end of the text. */
const SPEECH_OPEN = /["„“«»][^"“”«»\n]*$/;

/** Text with its spoken parts set apart. */
function spoken(text: string, at: number, ctx: Ctx, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  const say = (from: number, to: number) => {
    if (from > last) out.push(<Fragment key={`${key}t${last}`}>{leaf(text.slice(last, from), at + last, ctx)}</Fragment>);
    out.push(
      <span key={`${key}q${from}`} className="cv-say">
        {leaf(text.slice(from, to), at + from, ctx)}
      </span>,
    );
    last = to;
  };
  for (const m of text.matchAll(SPEECH)) say(m.index, m.index + m[0].length);
  if (ctx.open) {
    const m = SPEECH_OPEN.exec(text.slice(last));
    if (m) say(last + m.index, text.length);
  }
  if (last < text.length) out.push(<Fragment key={`${key}t${last}`}>{leaf(text.slice(last), at + last, ctx)}</Fragment>);
  return out;
}

const EMPHASIS = /\*\*[^*\n]+\*\*|\*[^*\n]+\*|`[^`\n]+`/g;
/** A single asterisk with text after it and no end yet, at the end of the text. */
const OPEN_EMPHASIS = /(^|[^*])\*([^*\s][^*\n]*)$/;

/** One line: `*emphasis*` and `**strong**` as elements, spoken words inside and around them. */
function line(text: string, at: number, ctx: Ctx, last: boolean): ReactNode[] {
  const out: ReactNode[] = [];
  let pos = 0;
  // Only the very end of the text can be a sentence still being spoken.
  const plain = (from: number, to: number) => {
    if (to > from) out.push(...spoken(text.slice(from, to), at + from, { ...ctx, open: ctx.open && last && to === text.length }, `s${from}`));
  };
  for (const m of text.matchAll(EMPHASIS)) {
    const strong = m[0].startsWith("**");
    const thought = m[0].startsWith("`");
    const mark = strong ? 2 : 1;
    if (m[0].length <= mark * 2) continue;
    plain(pos, m.index);
    const inner = spoken(m[0].slice(mark, -mark), at + m.index + mark, { ...ctx, open: false }, "e");
    out.push(
      strong ? (
        <strong key={`b${m.index}`}>{inner}</strong>
      ) : thought ? (
        // Thoughts in `backticks`: set apart like actions, in an ink of their own.
        <span key={`k${m.index}`} className="cv-think">
          {inner}
        </span>
      ) : (
        <em key={`i${m.index}`}>{inner}</em>
      ),
    );
    pos = m.index + m[0].length;
  }
  // An answer still being written: an `*action` whose end has not arrived reads as one already,
  // instead of showing its asterisk until it closes. Same key as the closed one, so it stays in place.
  const open = ctx.open && last ? OPEN_EMPHASIS.exec(text.slice(pos)) : null;
  if (open) {
    const star = pos + open.index + open[1].length;
    plain(pos, star);
    out.push(<em key={`i${star}`}>{spoken(text.slice(star + 1), at + star + 1, ctx, "e")}</em>);
    return out;
  }
  plain(pos, text.length);
  return out;
}

/** The paragraphs of a text with where each starts. */
function paragraphs(text: string): { text: string; at: number }[] {
  const out: { text: string; at: number }[] = [];
  let last = 0;
  for (const m of text.matchAll(/\n{2,}/g)) {
    out.push({ text: text.slice(last, m.index), at: last });
    last = m.index + m[0].length;
  }
  out.push({ text: text.slice(last), at: last });
  return out.filter((p) => p.text.trim());
}

type Props = {
  text: string;
  /** The model is writing this message. */
  streaming: boolean;
  /** Words of a search to set apart. */
  words?: string[] | null;
  /** New text fades in while the message is written (off: it appears at once). */
  animate?: boolean;
  /** Who writes it: named beside the dots before the first word, when the look asks for that. */
  who?: string;
};

export function Prose({ text, streaming, words = null, animate = false, who }: Props) {
  "use no memo"; // reads the clock and keeps the pieces still fading in a ref, on purpose
  // Text that was there when the message came on screen is never animated, only what arrives after.
  const seen = useRef(text.length);
  const fresh = useRef<Fresh[]>([]);
  const [, redraw] = useState(0);
  const now = performance.now();
  if (text.length < seen.current) {
    // The text was replaced by a shorter one (the finished answer, trimmed): nothing left to fade in.
    fresh.current = fresh.current.filter((f) => f.at < text.length);
  } else if (text.length > seen.current && animate && streaming) {
    fresh.current.push({ at: seen.current, born: now });
  }
  seen.current = text.length;
  // Pieces that have fully appeared become plain text again.
  if (fresh.current.length > 0) {
    const live = fresh.current.filter((f) => now - f.born < FADE_MS);
    fresh.current = animate ? live : [];
  }
  const pending = fresh.current.length;
  // The last pieces finish their fade after the answer is done: draw once more when they have.
  useEffect(() => {
    if (pending === 0 || streaming) return;
    const t = window.setTimeout(() => redraw((n) => n + 1), FADE_MS);
    return () => window.clearTimeout(t);
  }, [pending, streaming]);

  const ctx: Ctx = { words, fresh: fresh.current, now, open: streaming };
  const paras = paragraphs(text);
  return (
    <div className="prose">
      {paras.map((p, i) => {
        const lastPara = i === paras.length - 1;
        let at = p.at;
        return (
          <p key={i} className={streaming && lastPara ? "cursor" : undefined}>
            {p.text.split("\n").map((l, j, all) => {
              const start = at;
              at += l.length + 1;
              return (
                <Fragment key={j}>
                  {j > 0 && <br />}
                  {line(l, start, ctx, lastPara && j === all.length - 1)}
                </Fragment>
              );
            })}
          </p>
        );
      })}
      {streaming && paras.length === 0 && (
        <p className="cv-typing">
          <i />
          <i />
          <i />
          <span className="sr">{t("chat.typing")}</span>
          {who && (
            <span className="cv-typing-name" aria-hidden="true">
              {t("chat.typing.name", { name: who })}
            </span>
          )}
        </p>
      )}
    </div>
  );
}
