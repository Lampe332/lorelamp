// Windowing for long chats: only the messages near the visible part are in the page; the rest is
// held by two empty blocks of the same height. Short chats are drawn whole, exactly as before.
//
// Heights are measured as messages are drawn (ResizeObserver, margin included) and remembered by
// message id; messages never drawn count with the average. Moving content above the reader is
// corrected by the browser's scroll anchoring, as it already was with `content-visibility`.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";

/** Chats with more messages than this are windowed. */
export const WINDOW_FROM = 80;
/** Height of a message never drawn yet, before anything was measured (= `contain-intrinsic-size` of `.msg`). */
const ESTIMATE = 280;

/** Which messages are drawn. `follow`: the reader is at the end, so the window ends with the last message. */
type Range = { key: unknown; start: number; end: number; follow: boolean };

/** Largest index i (0 … n-1) whose message starts at or above `pos`. */
function indexAt(off: Float64Array, pos: number): number {
  let lo = 0;
  let hi = off.length - 2;
  if (hi < 0) return 0;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (off[mid] <= pos) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

/** Input that means the reader scrolls by hand. */
const INPUT = ["wheel", "touchstart", "pointerdown", "keydown"] as const;

/** How far beyond the visible part messages are kept drawn (px). */
const overscan = (view: number) => Math.max(view, 600);

/**
 * @param ids      message ids in order
 * @param key      changes when another chat opens (the window starts at the end again)
 * @param scroller the scrolling element
 * @param list     the element that holds the messages (and the two blocks) as its only children
 * @param follow   true while the reader is at the end and new text is followed
 */
export function useWindow(ids: readonly number[], key: unknown, scroller: RefObject<HTMLElement | null>, list: RefObject<HTMLElement | null>, follow: RefObject<boolean>) {
  const enabled = ids.length > WINDOW_FROM;
  const sizes = useRef(new Map<number, number>());
  // Height of a message not measured yet: the average of the first ones measured in this chat, then
  // kept, so the blocks do not change size as more messages are measured.
  const estimate = useRef<{ key: unknown; value: number } | null>(null);
  const [range, setRange] = useState<Range | null>(null);
  const shown = useRef<Range | null>(null); // what the last render drew
  const latest = useRef({ ids, key, enabled });
  latest.current = { ids, key, enabled };
  const jump = useRef<boolean | null>(null); // a scroll to the end waits for the render: smooth?
  // A smooth scroll to the end under way: where it was sent, and how it lands. Messages drawn on the
  // way can still grow the page, so on arrival it moves on to the real end.
  const gliding = useRef<{ target: number; land: () => void; stop: () => void } | null>(null);
  const observer = useRef<ResizeObserver | null>(null);
  const seen = useRef(new Map<Element, number>()); // drawn message elements → message id
  const reveal = useRef<number | null>(null); // a message to bring into view once it is drawn (`toIndex`)

  const offsets = useCallback((list: readonly number[]) => {
    const m = sizes.current;
    const e = estimate.current;
    const est = e && e.key === latest.current.key ? e.value : ESTIMATE;
    const off = new Float64Array(list.length + 1);
    for (let i = 0; i < list.length; i++) off[i + 1] = off[i] + (m.get(list[i]) ?? est);
    return off;
  }, []);

  const view = () => scroller.current?.clientHeight || window.innerHeight;
  const endStart = (off: Float64Array, h: number) => indexAt(off, off[off.length - 1] - h - overscan(h));
  /** The window around the message at `index` (a jump to it). */
  const around = (ids: readonly number[], key: unknown, index: number): Range => {
    const off = offsets(ids);
    const h = view();
    const ov = overscan(h);
    return { key, follow: false, start: indexAt(off, off[index] - ov), end: indexAt(off, off[index] + h + ov) };
  };
  /** A jump waits for its render: scroll events until then come from the old place. */
  const revealing = useCallback(() => reveal.current !== null, []);

  /** Moves the window when the part around the visible one is not drawn (or far too much is). */
  const update = useCallback(() => {
    const { ids, key, enabled } = latest.current;
    const el = scroller.current;
    const wrap = list.current;
    const cur = shown.current;
    if (!enabled || !el || !wrap || !cur) return;
    // A jump (`toIndex`) waits for its render: the scroll position still belongs to the old place,
    // so a window computed from it would replace the jump's and the message would never be drawn.
    if (reveal.current !== null) return;
    const n = ids.length;
    const off = offsets(ids);
    const h = el.clientHeight;
    const ov = overscan(h);
    const y = el.getBoundingClientRect().top - wrap.getBoundingClientRect().top; // visible top, in list coordinates
    const glide = gliding.current;
    if (glide && el.scrollTop >= Math.min(glide.target, el.scrollHeight - el.clientHeight) - 1) glide.land();
    const fol = follow.current || gliding.current !== null;
    const want = fol
      ? { start: indexAt(off, off[n] - h - ov), end: n - 1 }
      : { start: indexAt(off, y - ov), end: indexAt(off, y + h + ov) };
    const needTop = fol ? off[n] - h - ov / 2 : y - ov / 2;
    const needBottom = fol ? off[n] : y + h + ov / 2;
    const covered = off[cur.start] <= Math.max(0, needTop) && off[cur.end + 1] >= Math.min(off[n], needBottom);
    const tooMany = cur.end - cur.start > 3 * (want.end - want.start + 1) + 10;
    if (!covered || tooMany || cur.follow !== fol || cur.key !== key) setRange({ key, follow: fol, ...want });
  }, [offsets, scroller, list, follow]);

  // What this render draws.
  const n = ids.length;
  let start = 0;
  let end = n - 1;
  let before = 0;
  let after = 0;
  let drawn: Range | null = null;
  if (enabled) {
    const off = offsets(ids);
    if (range && range.key === key) {
      start = Math.min(range.start, n - 1);
      end = range.follow ? n - 1 : Math.min(Math.max(range.end, start), n - 1);
      drawn = { key, start, end, follow: range.follow };
    } else {
      start = endStart(off, view()); // a chat opens at its end
      drawn = { key, start, end, follow: true };
    }
    before = off[start];
    after = off[n] - off[end + 1];
  }

  useLayoutEffect(() => {
    shown.current = drawn;
    const wrap = list.current;
    const ro = observer.current;
    if (!drawn || !wrap) {
      if (ro) for (const el of seen.current.keys()) ro.unobserve(el);
      seen.current.clear();
      return;
    }
    if (!observer.current) {
      observer.current = new ResizeObserver((entries) => {
        let sum = 0;
        let count = 0;
        for (const e of entries) {
          const id = seen.current.get(e.target);
          if (id === undefined) continue;
          const box = e.borderBoxSize?.[0];
          const h = (box ? box.blockSize : e.target.getBoundingClientRect().height) + (parseFloat(getComputedStyle(e.target).marginBottom) || 0);
          sizes.current.set(id, h);
          sum += h;
          count += 1;
        }
        const key = latest.current.key;
        if (count && estimate.current?.key !== key) estimate.current = { key, value: sum / count };
        update();
      });
    }
    const obs = observer.current;
    const kids = wrap.children;
    const now = new Set<Element>();
    for (let i = drawn.start, k = before > 0 ? 1 : 0; i <= drawn.end && k < kids.length; i++, k++) {
      const el = kids[k];
      now.add(el);
      if (!seen.current.has(el)) obs.observe(el);
      seen.current.set(el, ids[i]);
    }
    for (const el of seen.current.keys()) {
      if (!now.has(el)) {
        obs.unobserve(el);
        seen.current.delete(el);
      }
    }
    const el = scroller.current;
    if (reveal.current !== null && el) {
      const id = reveal.current;
      const index = ids.indexOf(id);
      // Not drawn by this render (another range got in before the jump's): ask for the jump's again.
      if (index >= 0 && (drawn.follow || index < drawn.start || index > drawn.end)) setRange(around(ids, key, index));
      else {
        reveal.current = null;
        bringIntoView(el, wrap, id);
        // Messages drawn just now are measured in the next frames and can move it a little.
        requestAnimationFrame(() => requestAnimationFrame(() => bringIntoView(el, wrap, id)));
      }
    }
    // At the end, it stays there: the blocks can change size with each render.
    if (drawn.follow && follow.current && jump.current === null && el) el.scrollTop = el.scrollHeight;
    // A scroll to the end asked for by `toEnd`, now that the end is drawn.
    if (jump.current !== null && drawn.follow && el) {
      const smooth = jump.current;
      jump.current = null;
      gliding.current?.stop();
      if (smooth) {
        // From far away: jump to most of a screen above the end, glide the rest. The page between
        // is not drawn, so a long glide would show blank space.
        const endTop = el.scrollHeight - el.clientHeight;
        if (endTop - el.scrollTop > 0.75 * el.clientHeight) el.scrollTop = endTop - 0.75 * el.clientHeight;
        // Scrolling by hand ends the glide where it is.
        const stop = () => {
          for (const t of INPUT) el.removeEventListener(t, stop);
          gliding.current = null;
        };
        for (const t of INPUT) el.addEventListener(t, stop, { passive: true });
        gliding.current = {
          target: endTop,
          stop,
          land: () => {
            stop();
            el.scrollTop = el.scrollHeight;
          },
        };
        el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
      } else {
        el.scrollTop = el.scrollHeight;
      }
    }
  });

  // New or removed messages, another chat: check the window once the page has its new layout.
  useLayoutEffect(update, [ids, key, enabled, update]);

  useEffect(
    () => () => {
      observer.current?.disconnect();
      observer.current = null;
      seen.current.clear();
    },
    [],
  );

  /** Draws the end and scrolls there (the caller has set `follow`). */
  const toEnd = useCallback(
    (smooth: boolean) => {
      const { ids, key } = latest.current;
      const off = offsets(ids);
      reveal.current = null;
      jump.current = smooth;
      setRange({ key, follow: true, start: endStart(off, view()), end: ids.length - 1 });
    },
    [offsets], // eslint-disable-line react-hooks/exhaustive-deps
  );

  /** Draws the message at `index` and scrolls it into view (the caller has cleared `follow`). */
  const toIndex = useCallback(
    (index: number) => {
      const { ids, key, enabled } = latest.current;
      const el = scroller.current;
      const wrap = list.current;
      const id = ids[index];
      if (id === undefined || !el || !wrap) return;
      gliding.current?.stop();
      jump.current = null;
      if (!enabled) return bringIntoView(el, wrap, id);
      reveal.current = id;
      setRange(around(ids, key, index));
    },
    [offsets], // eslint-disable-line react-hooks/exhaustive-deps
  );

  return { enabled, start, end, before, after, update, toEnd, toIndex, revealing };
}

/** Scrolls so the message (`data-msg`) starts a fifth down the visible part, or shows whole if it is short. */
function bringIntoView(scroller: HTMLElement, list: HTMLElement, id: number) {
  const target = list.querySelector<HTMLElement>(`[data-msg="${id}"]`);
  if (!target) return;
  const box = target.getBoundingClientRect();
  const view = scroller.getBoundingClientRect();
  const room = scroller.clientHeight;
  const offset = box.height < room * 0.6 ? (room - box.height) / 2 : room * 0.2;
  scroller.scrollTop += box.top - view.top - offset;
}
