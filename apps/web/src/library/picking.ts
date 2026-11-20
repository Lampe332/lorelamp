// Picking cards in the characters tab without aiming at each checkbox: in the selection mode a frame
// dragged with the mouse picks every card it touches (it scrolls along at the top and bottom edge, Esc
// cancels it), and a long press with a finger on a card starts the selection with that card.

import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";

/** How far the mouse moves before a frame appears, so plain clicks stay clicks. */
const THRESHOLD = 6;
/** How long a finger rests on a card before it counts as a long press. */
const HOLD_MS = 500;
/** A finger that moves this far is scrolling, not holding. */
const HOLD_SLOP = 10;
/** The band at the top and bottom edge where the frame scrolls the page, and the fastest step per frame. */
const EDGE = 56;
const STEP = 22;

/** Presses on these keep their own meaning (a card's checkbox may start a frame like the card itself). */
const OWN = "a, button, select, textarea, input:not([type=checkbox]), [contenteditable], .lib-tools, .lib-selbar, .cc-menu";

const TEXT_INPUTS = new Set(["checkbox", "radio", "button", "submit", "reset", "range", "color", "file", "image"]);

/** Whether the focus is in a field that takes typed text (Ctrl+A belongs to the field then). */
export function typing(el: Element | null): boolean {
  if (el instanceof HTMLInputElement) return !TEXT_INPUTS.has(el.type);
  return el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement || (el instanceof HTMLElement && el.isContentEditable);
}

type Box = { left: number; top: number; right: number; bottom: number };
const touches = (a: Box, b: Box) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;

/** The element that scrolls the cards. */
function scrollerOf(el: Element): HTMLElement {
  for (let n: Element | null = el; n; n = n.parentElement) {
    const o = getComputedStyle(n).overflowY;
    if ((o === "auto" || o === "scroll") && n.scrollHeight > n.clientHeight) return n as HTMLElement;
  }
  return (document.scrollingElement ?? document.documentElement) as HTMLElement;
}

/** The visible part of the scroller; the selection bar sticks to its foot and covers the cards under it. */
function area(s: HTMLElement, root: HTMLElement): Box {
  const b = s === document.scrollingElement ? { left: 0, top: 0, right: innerWidth, bottom: innerHeight } : s.getBoundingClientRect();
  const bar = root.querySelector(".lib-selbar")?.getBoundingClientRect();
  return { left: b.left, top: b.top, right: b.right, bottom: bar && bar.top > b.top ? Math.min(b.bottom, bar.top) : b.bottom };
}

/** Swallows the click that follows a press used for a frame or a long press. */
function swallowClick(ms: number) {
  const off = () => {
    window.clearTimeout(timer);
    window.removeEventListener("click", stop, true);
    window.removeEventListener("pointerdown", off, true);
  };
  const stop = (e: Event) => {
    e.preventDefault();
    e.stopPropagation();
    off();
  };
  window.addEventListener("click", stop, true);
  window.addEventListener("pointerdown", off, true);
  const timer = window.setTimeout(off, ms);
}

type Drag = {
  pointer: number;
  /** Where the press began and where the pointer is now, in window coordinates. */
  x0: number;
  y0: number;
  x: number;
  y: number;
  /** The frame is showing (the pointer moved past the threshold). */
  on: boolean;
  /** Cancelled with Esc: the press still ends, but changes nothing. */
  cancelled: boolean;
  /** The selection before the frame, and whether the frame adds to it (Ctrl, Shift) or replaces it. */
  base: Set<number>;
  add: boolean;
  cards: { id: number; box: Box }[];
  scroller: HTMLElement;
  /** The scroller's position when the frame began: cards and frame are measured from there. */
  top0: number;
  /** The scroll position the frame was last drawn at. */
  seen: number;
  hits: string;
  raf: number;
};

type Options = {
  /** The tab's root, where presses are heard. */
  view: RefObject<HTMLDivElement | null>;
  /** The grid of cards (`.cc[data-id]`). */
  grid: RefObject<HTMLDivElement | null>;
  selecting: boolean;
  picked: Set<number> | null;
  setPicked: (next: Set<number>) => void;
  /** A long press on a card. */
  onHold: (id: number) => void;
};

/** Frame and long press over the cards; returns the ref for the frame element. */
export function useCardPicking(options: Options): RefObject<HTMLDivElement | null> {
  const frame = useRef<HTMLDivElement>(null);
  const live = useRef(options);
  useLayoutEffect(() => {
    live.current = options;
  });
  const { view: viewRef, grid: gridRef } = options;

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    let drag: Drag | null = null;
    let hold: { pointer: number; x: number; y: number; timer: number } | null = null;
    let heldAt = -Infinity;

    const showFrame = (on: boolean) => {
      if (frame.current) frame.current.hidden = !on;
      view.classList.toggle("x3-dragging", on);
    };

    const paint = () => {
      const d = drag;
      if (!d || !d.on || d.cancelled) return;
      const shift = d.scroller.scrollTop - d.top0;
      // In the coordinates of the moment the frame began, so the cards keep their place while it scrolls.
      const m = { left: Math.min(d.x0, d.x), right: Math.max(d.x0, d.x), top: Math.min(d.y0, d.y + shift), bottom: Math.max(d.y0, d.y + shift) };
      const a = area(d.scroller, view);
      const f = frame.current;
      if (f) {
        const left = Math.max(m.left, a.left);
        const top = Math.max(m.top - shift, a.top);
        f.style.left = `${left}px`;
        f.style.top = `${top}px`;
        f.style.width = `${Math.max(0, Math.min(m.right, a.right) - left)}px`;
        f.style.height = `${Math.max(0, Math.min(m.bottom - shift, a.bottom) - top)}px`;
      }
      const hits = d.cards.filter((c) => touches(c.box, m)).map((c) => c.id);
      const key = hits.join(",");
      if (key === d.hits) return;
      d.hits = key;
      const next = new Set(d.add ? d.base : []);
      for (const id of hits) next.add(id);
      live.current.setPicked(next);
    };

    // Near the top or bottom edge the page scrolls, faster the further out the pointer is.
    const tick = () => {
      const d = drag;
      if (!d || !d.on || d.cancelled) return;
      const a = area(d.scroller, view);
      const out = d.y < a.top + EDGE ? d.y - (a.top + EDGE) : d.y > a.bottom - EDGE ? d.y - (a.bottom - EDGE) : 0;      if (out !== 0) d.scroller.scrollTop += Math.sign(out) * Math.ceil(STEP * Math.min(1, Math.abs(out) / EDGE));
      // Scrolled here or by the wheel: the frame grows with the page.
      if (d.scroller.scrollTop !== d.seen) {
        d.seen = d.scroller.scrollTop;
        paint();
      }
      d.raf = requestAnimationFrame(tick);
    };

    const begin = (d: Drag) => {
      d.on = true;
      try {
        view.setPointerCapture(d.pointer);
      } catch {
        // The pointer is gone already; the window still hears its moves.
      }
      const grid = gridRef.current;
      d.cards = grid
        ? [...grid.querySelectorAll<HTMLElement>(".cc[data-id]")].map((el) => {
            const r = el.getBoundingClientRect();
            return { id: Number(el.dataset.id), box: { left: r.left, top: r.top, right: r.right, bottom: r.bottom } };
          })
        : [];
      d.scroller = scrollerOf(grid ?? view);
      d.top0 = d.scroller.scrollTop;
      d.seen = d.top0;
      window.getSelection()?.removeAllRanges();
      showFrame(true);
      d.raf = requestAnimationFrame(tick);
    };

    const stop = () => {
      if (!drag) return;
      cancelAnimationFrame(drag.raf);
      if (view.hasPointerCapture(drag.pointer)) view.releasePointerCapture(drag.pointer);
      showFrame(false);
      drag = null;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key, true);
    };

    const move = (e: PointerEvent) => {
      const d = drag;
      if (!d || e.pointerId !== d.pointer || d.cancelled) return;
      d.x = e.clientX;
      d.y = e.clientY;
      if (!d.on) {
        if (Math.hypot(d.x - d.x0, d.y - d.y0) < THRESHOLD) return;
        begin(d);
      }
      paint();
    };

    const up = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.pointer) return;
      if (drag.on) swallowClick(100);
      stop();
    };

    const cancel = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.pointer) return;
      if (drag.on && !drag.cancelled) live.current.setPicked(new Set(drag.base));
      stop();
    };

    const key = (e: KeyboardEvent) => {
      const d = drag;
      if (e.key !== "Escape" || !d || !d.on || d.cancelled) return;
      e.preventDefault();
      e.stopPropagation();
      d.cancelled = true;
      cancelAnimationFrame(d.raf);
      showFrame(false);
      live.current.setPicked(new Set(d.base));
    };

    const stopHold = () => {
      if (!hold) return;
      window.clearTimeout(hold.timer);
      hold = null;
      window.removeEventListener("pointermove", holdMove);
      window.removeEventListener("pointerup", stopHold);
      window.removeEventListener("pointercancel", stopHold);
    };

    const holdMove = (e: PointerEvent) => {
      if (hold && e.pointerId === hold.pointer && Math.hypot(e.clientX - hold.x, e.clientY - hold.y) > HOLD_SLOP) stopHold();
    };

    const startHold = (e: PointerEvent, id: number) => {
      hold = {
        pointer: e.pointerId,
        x: e.clientX,
        y: e.clientY,
        timer: window.setTimeout(() => {
          stopHold();
          heldAt = performance.now();
          // The finger lifts on a card that is a checkbox now; that tap must not undo the pick.
          swallowClick(1000);
          live.current.onHold(id);
        }, HOLD_MS),
      };
      window.addEventListener("pointermove", holdMove);
      window.addEventListener("pointerup", stopHold);
      window.addEventListener("pointercancel", stopHold);
    };

    const down = (e: PointerEvent) => {
      if (e.button !== 0 || drag || !e.isPrimary) return;
      stopHold();
      const t = e.target as Element;
      const grid = gridRef.current;
      if (!grid) return;
      if (e.pointerType === "touch") {
        const card = t.closest<HTMLElement>(".cc[data-id]");
        if (card && grid.contains(card) && !t.closest(".cc-star, .cc-menu")) startHold(e, Number(card.dataset.id));
        return;
      }
      if (!live.current.selecting || t.closest(OWN)) return;
      // The view's own scrollbar.
      if (t === view && e.clientX - view.getBoundingClientRect().left >= view.clientLeft + view.clientWidth) return;
      // No text selection, no picture drag; the click on a card still follows when the mouse stays put.
      e.preventDefault();
      if (typing(document.activeElement)) (document.activeElement as HTMLElement).blur();
      drag = {
        pointer: e.pointerId,
        x0: e.clientX,
        y0: e.clientY,
        x: e.clientX,
        y: e.clientY,
        on: false,
        cancelled: false,
        base: new Set(live.current.picked ?? []),
        add: e.ctrlKey || e.metaKey || e.shiftKey,
        cards: [],
        scroller: view,
        top0: 0,
        seen: 0,
        hits: "",
        raf: 0,
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      window.addEventListener("pointercancel", cancel);
      window.addEventListener("keydown", key, true);
    };

    // A long press opens no menu of the browser.
    const menu = (e: Event) => {
      if (hold || performance.now() - heldAt < 1500) e.preventDefault();
    };

    view.addEventListener("pointerdown", down);
    view.addEventListener("contextmenu", menu);
    return () => {
      stop();
      stopHold();
      view.removeEventListener("pointerdown", down);
      view.removeEventListener("contextmenu", menu);
    };
  }, [viewRef, gridRef]);

  return frame;
}
