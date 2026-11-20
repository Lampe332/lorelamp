// Opening one place in the settings (from the quick actions): the page and tab change through the
// hash as usual, then the place marked data-spot="…" scrolls into view, lights up for a moment and,
// when it holds a text field, gets the focus.

import { useEffect, useState } from "react";

let pending: string | null = null;
/** Keeps the place shown last in view while the page still grows; one at a time. */
let follow: ResizeObserver | null = null;
const EVENT = "rp-spot";

/** Goes to a settings tab (`#/settings`, `#/settings/logs`) and points at one place on it. */
export function openSetting(hash: string, spot: string) {
  pending = spot;
  if (location.hash !== hash) location.hash = hash;
  else window.dispatchEvent(new Event(EVENT));
}

/** A page whose places arrive later (loaded from the server) calls this once they are on screen,
 *  so a place asked for before is shown now. */
export const respot = () => window.dispatchEvent(new Event(EVENT));

/** For a place a page shows only when it is asked for (a hint): true from the moment the quick actions
 *  ask for `spot` (before or after the page is on screen) until `dismiss`. Read before `useSpot`
 *  takes the request, which happens two frames later. */
export function useSpotAsked(spot: string): [boolean, () => void] {
  const [asked, setAsked] = useState(() => pending === spot);
  useEffect(() => {
    const check = () => pending === spot && setAsked(true);
    window.addEventListener(EVENT, check);
    window.addEventListener("hashchange", check);
    return () => {
      window.removeEventListener(EVENT, check);
      window.removeEventListener("hashchange", check);
    };
  }, [spot]);
  // A request that was answered is used up, so the next page with this place does not see it again.
  return [
    asked,
    () => {
      if (pending === spot) pending = null;
      setAsked(false);
    },
  ];
}

/** In the settings page: shows the place asked for once it is on screen. `key` changes with the tab. */
export function useSpot(key: string) {
  useEffect(() => {
    let frame = 0;
    const show = () => {
      cancelAnimationFrame(frame);
      // Two frames: the quick actions window gives the focus back as it closes; this comes after.
      frame = requestAnimationFrame(() =>
        (frame = requestAnimationFrame(() => {
          if (!pending) return;
          const el = document.querySelector<HTMLElement>(`[data-spot="${pending}"]`);
          if (!el) return;
          pending = null;
          follow?.disconnect(); // the place shown before is no longer followed
          follow = null;
          // A place inside a folded section (a profile's sampler): unfold it first.
          for (let d = el.closest("details"); d; d = d.parentElement?.closest("details") ?? null) d.open = true;
          const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
          el.scrollIntoView({ block: "center", behavior: calm ? "auto" : "smooth" });
          // Parts above it may still be loading and push it out of view: follow it for a moment.
          const page = el.closest<HTMLElement>(".page");
          if (page) {
            let first = true; // a ResizeObserver reports once at the start
            const mine = new ResizeObserver(() => {
              if (first) return void (first = false);
              const r = el.getBoundingClientRect();
              if (r.bottom <= 0 || r.top >= innerHeight) el.scrollIntoView({ block: "center" });
            });
            mine.observe(page);
            follow = mine;
            window.setTimeout(() => mine.disconnect(), 1500);
          }
          el.classList.remove("spot-on");
          void el.offsetWidth; // restart the light when the same place is asked for twice
          el.classList.add("spot-on");
          window.setTimeout(() => el.classList.remove("spot-on"), 2400);
          // A place can name its own target (data-spot-focus); a choice gets the focus on its current option; a text field gets it with its text selected
          // (a long text only the caret at its end).
          const field =
            (el.matches("[data-spot-focus]") ? el : null) ??
            el.querySelector<HTMLElement>("[data-spot-focus]") ??
            el.querySelector<HTMLElement>('input:checked, [role="radio"][aria-checked="true"]') ??
            el.querySelector<HTMLElement>("input:not([disabled]), textarea:not([disabled])");
          if (field) {
            field.focus({ preventScroll: true });
            if (field instanceof HTMLInputElement && field.type !== "radio") field.select();
            else if (field instanceof HTMLTextAreaElement) field.setSelectionRange(field.value.length, field.value.length);
          }
        })),
      );
    };
    show();
    window.addEventListener(EVENT, show);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener(EVENT, show);
    };
  }, [key]);
}
