// The tablet layout's two swipes. Both follow the finger 1:1 while it is down (a direct transform,
// no state per move), decide on lifting by distance or by a flick, and settle with a transition
// that a new touch can interrupt.
//
// Variants of a reply: a sideways swipe on an answer that has several goes to the next (to the left)
// or the one before (to the right); at the first and last variant the text gives way only a little.
// On the chat's last reply a swipe to the left past its last variant writes a new one.
// The switch itself is the click on the answer's own arrows, so every rule of theirs (a writing
// model, the ends) holds here too.
//
// A pane over the page: it leaves the way it came in.

import { useEffect, useRef, type RefObject } from "react";

const START = 10; // px until a drag counts, and its direction is read
const FAR = 64; // px that switch on their own
const FLICK = 0.45; // px per ms that switch after a short way
const FLICK_MIN = 24;
const SETTLE = "transform .22s cubic-bezier(0.22, 1, 0.36, 1), opacity .16s linear";

type Drag = { id: number; x: number; y: number; prose: HTMLElement; msg: HTMLElement; sideways: boolean | null; lastX: number; lastT: number; speed: number; dx: number };

const usable = (b: HTMLButtonElement | null) => !!b && !b.disabled && b.getAttribute("aria-disabled") !== "true";
/** The button a swipe that way clicks. Past the last variant of the chat's last reply (or on a reply
 *  that has only one) a swipe to the left writes a new variant: "Neu generieren" (`.x7-regen`). */
const arrow = (msg: HTMLElement, dir: 1 | -1) => {
  const step = msg.querySelector<HTMLButtonElement>(dir === 1 ? ".x7-next" : ".x7-prev");
  return dir === 1 && !usable(step) ? (msg.querySelector<HTMLButtonElement>(".x7-regen") ?? step) : step;
};

/** A pane that came in from the right leaves the same way under a finger: it follows the finger 1:1,
 *  and past a third of its width or with a flick it goes (`onAway`), otherwise it settles back. Only
 *  while the pane lies over the page (`position: fixed`); a pane that is part of the page stays. */
export function useSwipeAway(pane: RefObject<HTMLElement | null>, onAway: () => void) {
  // The newest callback without subscribing again (a drag under way keeps its listeners).
  const latest = useRef(onAway);
  latest.current = onAway;
  useEffect(() => {
    const el = pane.current;
    if (!el) return;
    let drag: { id: number; x: number; y: number; sideways: boolean | null; lastX: number; lastT: number; speed: number; dx: number } | null = null;

    const down = (e: PointerEvent) => {
      if (e.pointerType !== "touch" || !e.isPrimary || drag || getComputedStyle(el).position !== "fixed") return;
      drag = { id: e.pointerId, x: e.clientX, y: e.clientY, sideways: null, lastX: e.clientX, lastT: e.timeStamp, speed: 0, dx: 0 };
    };
    const move = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      if (drag.sideways === null) {
        if (Math.abs(dx) < START && Math.abs(dy) < START) return;
        drag.sideways = Math.abs(dx) > Math.abs(dy) * 1.2;
        if (!drag.sideways) return void (drag = null);
        el.style.transition = "none";
      }
      const dt = e.timeStamp - drag.lastT;
      if (dt > 0) drag.speed = (e.clientX - drag.lastX) / dt;
      drag.lastX = e.clientX;
      drag.lastT = e.timeStamp;
      drag.dx = dx;
      // Against the way out there is nowhere to go: it gives way only a little.
      el.style.transform = `translateX(${Math.round(dx > 0 ? dx : dx * 0.15)}px)`;
    };
    const up = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      const d = drag;
      drag = null;
      if (!d.sideways) return;
      const away = e.type !== "pointercancel" && (d.dx >= el.offsetWidth / 3 || (d.dx >= FLICK_MIN && d.speed >= FLICK));
      if (away) {
        // The pane's own closing transition takes it out from where the finger left it: its timing
        // applies again before the page hears of the closing (so it stays visible on its way), and
        // the finger's offset goes once the page knows.
        el.style.transition = "";
        latest.current();
        requestAnimationFrame(() => requestAnimationFrame(() => (el.style.transform = "")));
        return;
      }
      el.style.transition = "transform .22s cubic-bezier(0.22, 1, 0.36, 1)";
      el.style.transform = "";
      const done = () => {
        el.style.transition = "";
        el.removeEventListener("transitionend", done);
      };
      el.addEventListener("transitionend", done);
    };

    // A sideways drag is ours alone: the browser makes no gesture of it (no fling whose end would
    // swallow the next tap).
    const own = (e: TouchEvent) => {
      if (drag?.sideways && e.cancelable) e.preventDefault();
    };
    el.addEventListener("pointerdown", down);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
    el.addEventListener("touchmove", own, { passive: false });
    return () => {
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      el.removeEventListener("touchmove", own);
      el.style.transition = "";
      el.style.transform = "";
    };
  }, [pane]);
}

export function useVariantSwipe(stream: RefObject<HTMLElement | null>, on: boolean) {
  useEffect(() => {
    const el = stream.current;
    if (!on || !el) return;
    let drag: Drag | null = null;
    const calm = matchMedia("(prefers-reduced-motion: reduce)");

    const down = (e: PointerEvent) => {
      if (e.pointerType !== "touch" || !e.isPrimary || drag) return;
      const target = e.target as HTMLElement;
      const prose = target.closest<HTMLElement>(".prose");
      const msg = prose?.closest<HTMLElement>(".msg");
      if (!prose || !msg || !msg.querySelector(".x7-swipe, .x7-regen") || target.closest("a, button, textarea, input")) return;
      drag = { id: e.pointerId, x: e.clientX, y: e.clientY, prose, msg, sideways: null, lastX: e.clientX, lastT: e.timeStamp, speed: 0, dx: 0 };
    };

    const move = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      if (drag.sideways === null) {
        if (Math.abs(dx) < START && Math.abs(dy) < START) return;
        drag.sideways = Math.abs(dx) > Math.abs(dy) * 1.2;
        if (!drag.sideways) return void (drag = null); // the page scrolls; this is not ours
        drag.prose.style.transition = "none";
      }
      const dt = e.timeStamp - drag.lastT;
      if (dt > 0) drag.speed = (e.clientX - drag.lastX) / dt;
      drag.lastX = e.clientX;
      drag.lastT = e.timeStamp;
      drag.dx = dx;
      // Nothing further that way: the text gives way only a little.
      const open = usable(arrow(drag.msg, dx < 0 ? 1 : -1));
      drag.prose.style.transform = `translateX(${Math.round(dx * (open ? 0.6 : 0.18))}px)`;
    };

    const settle = (prose: HTMLElement) => {
      prose.style.transition = SETTLE;
      prose.style.transform = "";
      prose.style.opacity = "";
      const done = () => {
        prose.style.transition = "";
        prose.removeEventListener("transitionend", done);
      };
      prose.addEventListener("transitionend", done);
    };

    const up = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      const d = drag;
      drag = null;
      if (!d.sideways) return;
      const dir: 1 | -1 = d.dx < 0 ? 1 : -1;
      const far = Math.abs(d.dx) >= FAR || (Math.abs(d.dx) >= FLICK_MIN && Math.abs(d.speed) >= FLICK && Math.sign(d.speed) === Math.sign(d.dx));
      const button = arrow(d.msg, dir);
      if (e.type === "pointercancel" || !far || !usable(button)) return settle(d.prose);
      button!.click();
      if (calm.matches) return settle(d.prose);
      // The old text leaves the way the finger went; the new one comes in from the other side.
      d.prose.style.transition = "transform .14s ease-in, opacity .14s linear";
      d.prose.style.transform = `translateX(${-dir * 48}px)`;
      d.prose.style.opacity = "0";
      window.setTimeout(() => {
        d.prose.style.transition = "none";
        d.prose.style.transform = `translateX(${dir * 32}px)`;
        void d.prose.offsetWidth; // the start of the way back, before it moves
        settle(d.prose);
      }, 150);
    };

    // A sideways drag is ours alone: the browser makes no gesture of it (no fling whose end would
    // swallow the next tap).
    const own = (e: TouchEvent) => {
      if (drag?.sideways && e.cancelable) e.preventDefault();
    };
    el.addEventListener("pointerdown", down);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
    el.addEventListener("touchmove", own, { passive: false });
    return () => {
      el.removeEventListener("pointerdown", down);
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      el.removeEventListener("touchmove", own);
      if (drag) settle(drag.prose);
    };
  }, [stream, on]);
}
