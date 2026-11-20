// How a chat looks: the reading settings (face, size, line height, width, how messages are set),
// app-wide and per chat, and the backdrop picture. Types, the server calls, and what the page gets
// from them (CSS variables and data attributes on the chat's root).

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import { api, apiUrl, postJson, token } from "../api";
import { dateFormat, texts } from "../i18n";

export type FontId = "sitka" | "georgia" | "palatino" | "cambria" | "sans" | "verdana" | "mono";
export type StyleId = "book" | "bubbles" | "sides" | "wide" | "compact";
/** An ink for spoken words or actions: the character's colour, the story's own ink, the quiet ink
 *  of narration, or a hue (`h0` … `h359`; the theme decides how light it is, so it reads in all six). */
export type Ink = "figure" | "text" | "soft" | `h${number}`;
/** What a bubble is filled with: the character's colour, the theme's plain surface, nothing (the
 *  text stands on the page), or a hue (as light as the theme's character fields). */
export type Fill = "figure" | "plain" | "none" | `h${number}`;
export type AvShape = "round" | "soft" | "square";
export type Enter = "none" | "fade" | "rise" | "pop";
export type Typing = "dots" | "name" | "none";

/** Reading settings. An empty field follows the level above: a chat the app, the app its defaults. */
export type Look = {
  font?: FontId;
  size?: number;
  line?: number;
  width?: number;
  style?: StyleId;
  justify?: boolean;
  speech?: boolean;
  animate?: boolean;
  timings?: boolean;
  /** Ink of spoken words ("…"). */
  say?: Ink;
  say_bold?: boolean;
  /** Ink of actions (*…*). */
  act?: Ink;
  act_italic?: boolean;
  /** The inks in your own messages too. */
  mine?: boolean;
  /** Pictures at the messages. */
  avatars?: boolean;
  /** Space between paragraphs in em. */
  para?: number;
  /** Ink of plain narration (outside quotes and asterisks). */
  story?: Ink;
  /** Ink of thoughts (`…` in backticks). */
  think?: Ink;
  /** Widest a bubble grows, percent of its row. */
  bubble?: number;
  /** "Messenger breit": space to the chat's edges, px. */
  edge?: number;
  /** Space below each message, px. */
  gap?: number;
  /** Roundness of the bubbles, px. */
  corner?: number;
  /** The corner next to the speaker is pointed. */
  tail?: boolean;
  shadow?: boolean;
  /** How much the bubbles cover what is behind them, percent. */
  fill?: number;
  /** Blur of a backdrop behind the bubbles, px. */
  frost?: number;
  ai_fill?: Fill;
  me_fill?: Fill;
  /** Size of the pictures at the messages, px. */
  av_size?: number;
  av_shape?: AvShape;
  names?: boolean;
  times?: boolean;
  /** Both-sides layouts: yours left, theirs right. */
  flip?: boolean;
  enter?: Enter;
  typing?: Typing;
};

/** The visible part of a backdrop (percent), how much the theme's ground covers it (percent), blur in px. */
export type Frame = { x: number; y: number; dim: number; blur: number };
/** `preset`: the ready-made picture it is a copy of (see presets.ts), if it is one. */
export type Backdrop = Frame & { rev: number; preset?: string };
export type BackdropMode = "character" | "own" | "none";

export type ChatLook = {
  character_id: number;
  global: Look;
  look: Look;
  backdrop: { mode: BackdropMode; own: Backdrop | null; character: Backdrop | null };
};

/** The faces on offer. All of them ship with the device (Windows first, then Apple, then Android);
 *  nothing is downloaded. `stack` empty = the app's own reading face. */
export const FONTS: { id: FontId; name: string; note: string; stack: string }[] = [
  texts({ id: "sitka", name: "Sitka", stack: "" }, { note: "chat.font.sitka" }),
  texts({ id: "georgia", name: "Georgia", stack: 'Georgia, "Times New Roman", "Noto Serif", serif' }, { note: "chat.font.georgia" }),
  texts({ id: "palatino", name: "Palatino", stack: '"Palatino Linotype", Palatino, "Book Antiqua", "Iowan Old Style", "Noto Serif", serif' }, { note: "chat.font.palatino" }),
  texts({ id: "cambria", name: "Cambria", stack: 'Cambria, "Hoefler Text", "Noto Serif", Georgia, serif' }, { note: "chat.font.cambria" }),
  texts({ id: "sans", name: "Segoe", stack: '"Segoe UI Variable Text", "Segoe UI", system-ui, -apple-system, Roboto, sans-serif' }, { note: "chat.font.sans" }),
  texts({ id: "verdana", name: "Verdana", stack: 'Verdana, "DejaVu Sans", "Segoe UI", system-ui, sans-serif' }, { note: "chat.font.verdana" }),
  texts({ id: "mono", name: "Consolas", stack: 'Consolas, "Cascadia Mono", "SF Mono", Menlo, "Roboto Mono", monospace' }, { note: "chat.font.mono" }),
];

export const STYLES: { id: StyleId; name: string; note: string }[] = [
  texts({ id: "book" }, { name: "chat.style.book", note: "chat.style.book.note" }),
  texts({ id: "bubbles" }, { name: "chat.style.bubbles", note: "chat.style.bubbles.note" }),
  texts({ id: "sides" }, { name: "chat.style.sides", note: "chat.style.sides.note" }),
  texts({ id: "wide" }, { name: "chat.style.wide", note: "chat.style.wide.note" }),
  texts({ id: "compact" }, { name: "chat.style.compact", note: "chat.style.compact.note" }),
];

/** The layouts with both sides as bubbles at their own edge. */
export const twoSided = (style: StyleId | undefined) => style === "sides" || style === "wide";

export const AV_SHAPES: { id: AvShape; name: string }[] = [
  texts({ id: "round" }, { name: "chat.look.avShape.round" }),
  texts({ id: "soft" }, { name: "chat.look.avShape.soft" }),
  texts({ id: "square" }, { name: "chat.look.avShape.square" }),
];
export const ENTERS: { id: Enter; name: string }[] = [
  texts({ id: "none" }, { name: "chat.look.enter.none" }),
  texts({ id: "fade" }, { name: "chat.look.enter.fade" }),
  texts({ id: "rise" }, { name: "chat.look.enter.rise" }),
  texts({ id: "pop" }, { name: "chat.look.enter.pop" }),
];
export const TYPINGS: { id: Typing; name: string }[] = [
  texts({ id: "dots" }, { name: "chat.look.typing.dots" }),
  texts({ id: "name" }, { name: "chat.look.typing.name" }),
  texts({ id: "none" }, { name: "chat.look.typing.none" }),
];

export const SIZE = { min: 15, max: 26, step: 1 };
export const LINE = { min: 1.3, max: 2.1, step: 0.05 };
export const WIDTH = { min: 520, max: 1600, step: 20 };
export const PARA = { min: 0, max: 1.6, step: 0.05 };
export const BUBBLE = { min: 40, max: 100, step: 1 };
export const EDGE = { min: 0, max: 160, step: 2 };
export const GAP = { min: 0, max: 60, step: 2 };
export const CORNER = { min: 0, max: 32, step: 1 };
export const FILL = { min: 10, max: 100, step: 5 };
export const FROST = { min: 0, max: 20, step: 1 };
export const AV_SIZE = { min: 16, max: 64, step: 2 };

/** The hues on offer as swatches (any other one comes from the hue slider). */
export const HUES: { hue: number; name: string }[] = [
  texts({ hue: 5 }, { name: "chat.hue.red" }),
  texts({ hue: 30 }, { name: "chat.hue.orange" }),
  texts({ hue: 50 }, { name: "chat.hue.gold" }),
  texts({ hue: 140 }, { name: "chat.hue.green" }),
  texts({ hue: 180 }, { name: "chat.hue.teal" }),
  texts({ hue: 215 }, { name: "chat.hue.blue" }),
  texts({ hue: 270 }, { name: "chat.hue.violet" }),
  texts({ hue: 325 }, { name: "chat.hue.pink" }),
];

/** The hue of an ink, if it is one. */
export const inkHue = (ink: Ink | undefined): number | null => (ink && /^h\d{1,3}$/.test(ink) ? Number(ink.slice(1)) : null);

/** An ink as a CSS colour. Everything comes from the theme's tokens: a hue takes the theme's
 *  lightness for names, so it stays readable on a dark and on a light ground. */
export function inkColor(ink: Ink): string {
  const hue = inkHue(ink);
  if (hue !== null) return `hsl(${hue} var(--hue-name))`;
  return ink === "text" ? "var(--story)" : ink === "soft" ? "var(--narration)" : "hsl(var(--h, 30) var(--hue-name))";
}

/** A bubble's fill as a CSS colour, from the theme's tokens (a hue takes the lightness of the
 *  theme's character fields, so text on it reads in all six themes). */
export function fillColor(fill: Fill): string {
  const hue = inkHue(fill as Ink);
  if (hue !== null) return `hsl(${hue} var(--hue-field))`;
  return fill === "plain" ? "var(--surface)" : fill === "none" ? "transparent" : "hsl(var(--h, 30) var(--hue-field))";
}

/** The app's own defaults. Size and width have none: they follow the window (see style.css); the
 *  bubbles' width, corners and the space between messages follow the layout (see chat.css). */
export const DEFAULTS = {
  font: "sitka",
  line: 1.68,
  style: "book",
  justify: false,
  speech: true,
  animate: true,
  timings: true,
  say: "figure",
  say_bold: false,
  act: "soft",
  act_italic: true,
  mine: false,
  avatars: true,
  para: 0.85,
  story: "text",
  think: "soft",
  tail: true,
  shadow: false,
  fill: 100,
  frost: 0,
  ai_fill: "figure",
  me_fill: "plain",
  av_shape: "round",
  names: true,
  times: false,
  flip: false,
  enter: "none",
  typing: "dots",
} as const satisfies Look;

/** `over` on top of `base`: every field `over` sets wins. */
export const merge = (base: Look, over: Look): Look => {
  const out: Look = { ...base };
  for (const [k, v] of Object.entries(over)) if (v !== undefined && v !== null) (out as Record<string, unknown>)[k] = v;
  return out;
};

/** A look without its empty fields (what is stored). */
export const tidy = (look: Look): Look => merge({}, look);

/** What the page draws: defaults, then the app's settings, then the chat's. */
export const resolve = (global: Look, chat: Look = {}) => merge(merge(DEFAULTS, global), chat);

/** The look as CSS variables and data attributes for the chat's root element. */
export function lookProps(look: Look): { style: CSSProperties; data: Record<string, string> } {
  const style: Record<string, string> = {};
  const stack = FONTS.find((f) => f.id === look.font)?.stack;
  if (stack) style["--read"] = stack;
  if (look.size) style["--read-size"] = `${look.size}px`;
  if (look.line) style["--read-lh"] = String(look.line);
  if (look.width) style["--measure"] = `${look.width}px`;
  if (look.say && look.say !== DEFAULTS.say) style["--cv-say"] = inkColor(look.say);
  if (look.act && look.act !== DEFAULTS.act) style["--cv-act"] = inkColor(look.act);
  if (look.para !== undefined && look.para !== DEFAULTS.para) style["--cv-para"] = `${look.para}em`;
  if (look.story && look.story !== DEFAULTS.story) style["--cv-story"] = inkColor(look.story);
  if (look.think && look.think !== DEFAULTS.think) style["--cv-think"] = inkColor(look.think);
  if (look.bubble !== undefined) style["--cv-bubble"] = `${look.bubble}%`;
  if (look.edge !== undefined) style["--cv-edge"] = `${look.edge}px`;
  if (look.gap !== undefined) style["--cv-gap"] = `${look.gap}px`;
  if (look.av_size !== undefined) style["--cv-av"] = `${look.av_size}px`;
  // Corners: only drawn by the look's own rule once it sets them; until then each layout keeps its own.
  const tail = look.tail ?? DEFAULTS.tail;
  const shaped = look.corner !== undefined || !tail;
  if (shaped) {
    const r = look.corner ?? 18;
    style["--cv-r"] = `${r}px`;
    style["--cv-rt"] = `${tail ? Math.min(r, Math.max(3, Math.round(r * 0.28))) : r}px`;
  }
  // Fills: a colour, made see-through by `fill` (the part of it that covers what is behind).
  const cover = look.fill ?? DEFAULTS.fill;
  const bubbleBg = (fill: Fill, plain: Fill) =>
    fill === plain && cover >= 100 ? undefined : cover >= 100 || fill === "none" ? fillColor(fill) : `color-mix(in srgb, ${fillColor(fill)} ${cover}%, transparent)`;
  const ai = bubbleBg(look.ai_fill ?? DEFAULTS.ai_fill, DEFAULTS.ai_fill);
  const me = bubbleBg(look.me_fill ?? DEFAULTS.me_fill, DEFAULTS.me_fill);
  if (ai) style["--cv-ai-bg"] = ai;
  if (me) style["--cv-me-bg"] = me;
  const frost = look.frost ?? DEFAULTS.frost;
  if (frost > 0) style["--cv-frost"] = `${frost}px`;
  const on = (key: "speech" | "timings" | "say_bold" | "act_italic" | "mine" | "avatars" | "shadow" | "names" | "times" | "flip") => ((look[key] ?? DEFAULTS[key]) ? "on" : "off");
  return {
    style: style as CSSProperties,
    data: {
      "data-cv-style": look.style ?? DEFAULTS.style,
      "data-cv-justify": look.justify ? "on" : "off",
      "data-cv-speech": on("speech"),
      "data-cv-timings": on("timings"),
      "data-cv-say-bold": on("say_bold"),
      "data-cv-act-italic": on("act_italic"),
      "data-cv-mine": on("mine"),
      "data-cv-avatars": on("avatars"),
      "data-cv-shadow": on("shadow"),
      "data-cv-names": on("names"),
      "data-cv-times": on("times"),
      "data-cv-flip": on("flip"),
      "data-cv-shape": shaped ? "on" : "off",
      "data-cv-bubble": look.bubble !== undefined ? "on" : "off",
      "data-cv-gap": look.gap !== undefined ? "on" : "off",
      "data-cv-frost": frost > 0 ? "on" : "off",
      "data-cv-av-size": look.av_size !== undefined ? "on" : "off",
      "data-cv-av-shape": look.av_shape ?? DEFAULTS.av_shape,
      "data-cv-ai-fill": (look.ai_fill ?? DEFAULTS.ai_fill) === "none" ? "none" : "on",
      "data-cv-me-fill": (look.me_fill ?? DEFAULTS.me_fill) === "none" ? "none" : "on",
      "data-cv-enter": look.enter ?? DEFAULTS.enter,
      "data-cv-typing": look.typing ?? DEFAULTS.typing,
    },
  };
}

/** When a message was written, as the chat shows it beside the name: the time of day, with the
 *  date in front when it was not today. */
export function messageTime(at: number, now = new Date()): string {
  const d = new Date(at * 1000);
  const today = d.toDateString() === now.toDateString();
  return today
    ? dateFormat({ hour: "2-digit", minute: "2-digit" }).format(d)
    : dateFormat({ day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", ...(d.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }) }).format(d);
}

const picture = (path: string, rev: number) => apiUrl(`${path}?token=${encodeURIComponent(token)}&v=${rev}`);
export const chatBackdropUrl = (chatId: number, rev: number) => picture(`/chats/${chatId}/backdrop`, rev);
export const characterBackdropUrl = (characterId: number, rev: number) => picture(`/characters/${characterId}/backdrop`, rev);

/** The backdrop a chat shows now (its own, the character's, or none) with its address. */
export function shownBackdrop(chatId: number, view: ChatLook | null): (Backdrop & { url: string; of: "own" | "character" }) | null {
  if (!view || view.backdrop.mode === "none") return null;
  const { own, character } = view.backdrop;
  if (view.backdrop.mode === "own" && own) return { ...own, url: chatBackdropUrl(chatId, own.rev), of: "own" };
  if (character) return { ...character, url: characterBackdropUrl(view.character_id, character.rev), of: "character" };
  return null;
}

export const getLook = () => api<{ look: Look }>("/chat-look").then((r) => r.look);
export const putLook = (look: Look) => postJson("/chat-look", tidy(look), "PUT");
export const getChatLook = (chatId: number) => api<ChatLook>(`/chats/${chatId}/look`);
export const putChatLook = (chatId: number, body: { look?: Look; backdrop?: BackdropMode }) =>
  postJson(`/chats/${chatId}/look`, body.look ? { ...body, look: tidy(body.look) } : body, "PUT");
export const putChatBackdrop = (chatId: number, file: Blob) => api(`/chats/${chatId}/backdrop`, { method: "PUT", body: file });
export const deleteChatBackdrop = (chatId: number) => api(`/chats/${chatId}/backdrop`, { method: "DELETE" });
export const putChatBackdropFrame = (chatId: number, frame: Frame) => postJson(`/chats/${chatId}/backdrop/frame`, frame, "PUT");
export const backdropToCharacter = (chatId: number) => postJson(`/chats/${chatId}/backdrop/to-character`, {});
export const deleteCharacterBackdrop = (characterId: number) => api(`/characters/${characterId}/backdrop`, { method: "DELETE" });
export const putCharacterBackdropFrame = (characterId: number, frame: Frame) => postJson(`/characters/${characterId}/backdrop/frame`, frame, "PUT");

/** Another device (or this one) changed how chats look: `{"type":"look"}` on the socket. */
export const LOOK_EVENT = "rp-look";
export const tellLookChanged = () => window.dispatchEvent(new Event(LOOK_EVENT));

/** Runs `save` a moment after the last call, so dragging a slider sends one request, not sixty.
 *  `busy()` is true from the first call until the last save has answered. */
function useLater() {
  const timer = useRef<number | undefined>(undefined);
  const open = useRef(0);
  const later = useCallback((save: () => Promise<unknown>, done: () => void) => {
    if (timer.current === undefined) open.current += 1;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = undefined;
      void save()
        .catch(() => {})
        .finally(() => {
          open.current -= 1;
          done();
        });
    }, 220);
  }, []);
  const busy = useCallback(() => open.current > 0, []);
  return { later, busy };
}

/** The app-wide reading settings (the settings tab). `look` is null until they arrived. */
export function useGlobalLook() {
  const [look, setLook] = useState<Look | null>(null);
  const [failed, setFailed] = useState(false);
  const { later, busy } = useLater();
  const load = useCallback(() => {
    if (busy()) return; // our own change is still on its way
    getLook().then(
      (l) => !busy() && (setLook(l), setFailed(false)),
      () => setFailed(true),
    );
  }, [busy]);
  useEffect(() => {
    load();
    window.addEventListener(LOOK_EVENT, load);
    return () => window.removeEventListener(LOOK_EVENT, load);
  }, [load]);
  const change = (next: Look) => {
    const tidied = tidy(next);
    setLook(tidied);
    later(() => putLook(tidied), load);
  };
  return { look, failed, change };
}

/** How the chat on screen looks. `view` is null until it arrived (the chat then shows the app's
 *  defaults); every change is drawn at once and saved a moment later. */
export function useChatLook(chatId: number | undefined) {
  const [state, setState] = useState<{ chatId: number; view: ChatLook } | null>(null);
  const { later: laterLook, busy: busyLook } = useLater();
  const { later: laterFrame, busy: busyFrame } = useLater();
  const busy = useCallback(() => busyLook() || busyFrame(), [busyLook, busyFrame]);
  const load = useCallback(() => {
    if (chatId === undefined || busy()) return;
    getChatLook(chatId).then(
      (view) => !busy() && setState({ chatId, view }),
      () => {},
    );
  }, [chatId, busy]);
  useEffect(() => {
    load();
    window.addEventListener(LOOK_EVENT, load);
    return () => window.removeEventListener(LOOK_EVENT, load);
  }, [load]);
  const view = state && state.chatId === chatId ? state.view : null;

  /** What this chat changes of the reading settings (the whole set). */
  const setLook = (look: Look) => {
    if (!view || chatId === undefined) return;
    const tidied = tidy(look);
    setState({ chatId, view: { ...view, look: tidied } });
    laterLook(() => putChatLook(chatId, { look: tidied }), load);
  };
  /** Moves, dims or softens the backdrop on show (the chat's own or the character's default). */
  const setFrame = (frame: Frame) => {
    if (!view || chatId === undefined) return;
    const { mode, own, character } = view.backdrop;
    if (mode === "own" && own) {
      setState({ chatId, view: { ...view, backdrop: { ...view.backdrop, own: { ...own, ...frame } } } });
      laterFrame(() => putChatBackdropFrame(chatId, frame), load);
    } else if (mode === "character" && character) {
      setState({ chatId, view: { ...view, backdrop: { ...view.backdrop, character: { ...character, ...frame } } } });
      laterFrame(() => putCharacterBackdropFrame(view.character_id, frame), load);
    }
  };
  return { view, setLook, setFrame, reload: load };
}

/** The reader asked the system for less motion. */
export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  useEffect(() => {
    const q = window.matchMedia("(prefers-reduced-motion: reduce)");
    const on = () => setReduced(q.matches);
    q.addEventListener("change", on);
    return () => q.removeEventListener("change", on);
  }, []);
  return reduced;
}

/** From this width of the chat on (story plus picture, px) the large picture stands beside the
 *  story; below it is a band above. The chat's own width counts, not the window's: the sidebar takes
 *  its share, so an 870 px window leaves the chat about 650 px. */
export const STAGE_SIDE_MIN = 600;

/** Where the large picture goes in this frame: "side" or "band" (see chat.css `data-cv-stage`).
 *  `key` changes when the frame element does. */
export function useStageSide(frame: RefObject<HTMLElement | null>, key: unknown): "side" | "band" {
  const [side, setSide] = useState(true);
  useLayoutEffect(() => {
    const el = frame.current;
    if (!el) return;
    const read = () => setSide(el.clientWidth >= STAGE_SIDE_MIN);
    read();
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [frame, key]);
  return side ? "side" : "band";
}

/** Whether the character's picture is shown large next to the story; kept per device. */
const STAGE_KEY = "rp-chat-stage";
export function useStage(): [boolean, (on: boolean) => void] {
  const [on, setOn] = useState(() => {
    try {
      return localStorage.getItem(STAGE_KEY) === "1";
    } catch {
      return false;
    }
  });
  const set = (next: boolean) => {
    setOn(next);
    try {
      localStorage.setItem(STAGE_KEY, next ? "1" : "0");
    } catch {
      // Blocked storage: the choice lives as long as the page.
    }
  };
  return [on, set];
}
