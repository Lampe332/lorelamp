// Small shared pieces: icons, avatars, the brand mark.

import { useId, useState, type CSSProperties } from "react";
import { avatarUrl, personaAvatarUrl, thumbUrl } from "./api";

const PATHS = {
  users:
    "M16 19v-1.5a3.5 3.5 0 0 0-3.5-3.5h-5A3.5 3.5 0 0 0 4 17.5V19M10 10.5a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM20 19v-1.5a3.5 3.5 0 0 0-2.5-3.35M15 4.65a3 3 0 0 1 0 5.7",
  plus: "M12 5v14M5 12h14",
  send: "M12 19V5M6 11l6-6 6 6",
  stop: "M7.5 7.5h9v9h-9z",
  back: "M15 5l-7 7 7 7",
  trash: "M4.5 7h15M10 7V4.5h4V7M6.5 7l.8 12h9.4l.8-12M10 10.5v5M14 10.5v5",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14ZM20 20l-4-4",
  close: "M6 6l12 12M18 6L6 18",
  upload: "M12 15V4M7.5 8.5 12 4l4.5 4.5M5 15v3.5h14V15",
  chat: "M5 6.5h14v9.5H11l-4 3.5V16H5z",
  sliders: "M5 8h6M15 8h4M5 16h3M12 16h7M13 5.5v5M10 13.5v5",
  home: "M4.5 11 12 4.5l7.5 6.5M6.5 9.5V19h11V9.5M10 19v-5h4v5",
  pencil: "M5 19h3.5L18.5 9 15 5.5 5 15.5zM13.5 7l3.5 3.5",
  copy: "M9 9h10.5v10.5H9zM5 15V4.5h10.5",
  check: "M5 12.5l4.5 4.5L19 7.5",
  rewind: "M4 4.5v5h5M4 9.5l2.6-2.6A8 8 0 1 1 4.6 15",
  fork: "M7 8a2 2 0 1 0 0-4 2 2 0 0 0 0 4ZM17 8a2 2 0 1 0 0-4 2 2 0 0 0 0 4ZM12 20a2 2 0 1 0 0-4 2 2 0 0 0 0 4ZM7 8v1.5A2.5 2.5 0 0 0 9.5 12h5A2.5 2.5 0 0 0 17 9.5V8M12 12v4",
  gear: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM10.3 3.5h3.4l.5 2.3 1.6.9 2.2-.8 1.7 2.9-1.7 1.6v1.9l1.7 1.6-1.7 2.9-2.2-.8-1.6.9-.5 2.3h-3.4l-.5-2.3-1.6-.9-2.2.8-1.7-2.9 1.7-1.6v-1.9L4.3 8.8 6 5.9l2.2.8 1.6-.9z",
  list: "M9 6.5h10M9 12h10M9 17.5h10M5 6.5h.01M5 12h.01M5 17.5h.01",
  more: "M5.2 12a.8.8 0 1 0 1.6 0 .8.8 0 1 0-1.6 0ZM11.2 12a.8.8 0 1 0 1.6 0 .8.8 0 1 0-1.6 0ZM17.2 12a.8.8 0 1 0 1.6 0 .8.8 0 1 0-1.6 0Z",
  select: "M5 5h14v14H5zM8.5 12.2l2.4 2.4 4.6-5.1",
  star: "M12 4.2l2.4 4.9 5.4.8-3.9 3.8.9 5.4L12 16.6l-4.8 2.5.9-5.4-3.9-3.8 5.4-.8z",
  tag: "M4.5 4.5h7l8 8-7 7-8-8zM8.5 8.5h.01",
  download: "M12 4v11M7.5 10.5 12 15l4.5-4.5M5 19.5h14",
  down: "M12 5v14M6 13l6 6 6-6",
  next: "M5 12h14M13 6l6 6-6 6",
  card: "M6.5 4.5h11a1.5 1.5 0 0 1 1.5 1.5v12a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 18V6a1.5 1.5 0 0 1 1.5-1.5ZM8.5 9h7M8.5 12.5h7M8.5 16h4",
  // Places that come later ("Bald") and the quick actions.
  book: "M5 5.5A1.5 1.5 0 0 1 6.5 4H12v15.5H6.5A1.5 1.5 0 0 1 5 18zM12 4h5.5A1.5 1.5 0 0 1 19 5.5V18a1.5 1.5 0 0 1-1.5 1.5H12M7.5 8h2M14.5 8h2",
  persona: "M4.5 6h15v12h-15zM9 13a2 2 0 1 0 0-4 2 2 0 0 0 0 4ZM6.5 15.5a2.8 2.8 0 0 1 5 0M14 10h3M14 13h3",
  group: "M3.5 5.5h11v8H8.5l-3 2.5v-2.5h-2zM14.5 9h6v8h-2v2.5l-3-2.5H10v-3.5",
  model: "M7 7h10v10H7zM10 10h4v4h-4zM10 3.5V7M14 3.5V7M10 17v3.5M14 17v3.5M3.5 10H7M3.5 14H7M17 10h3.5M17 14h3.5",
  chart: "M4.5 19.5h15M7 16v-5M12 16V6M17 16v-8",
  qr: "M4.5 4.5h6v6h-6zM13.5 4.5h6v6h-6zM4.5 13.5h6v6h-6zM13.5 13.5h2.5V16h-2.5zM17 17h2.5v2.5H17zM7.5 7.5h.01M16.5 7.5h.01M7.5 16.5h.01",
  memory: "M19.5 12A7.5 7.5 0 1 1 17 6.4M12 8v4l2.5 2.5M19.5 4v3.5H16",
  palette:
    "M12 4.5a7.5 7.5 0 1 0 0 15c.9 0 1.5-.6 1.5-1.4 0-.4-.2-.8-.4-1.1-.2-.3-.4-.7-.4-1.1 0-.8.7-1.4 1.5-1.4H16a3.5 3.5 0 0 0 3.5-3.5c0-3.7-3.4-6.5-7.5-6.5ZM8 12h.01M10 8.5h.01M14.5 8.5h.01",
  chevron: "M7 10l5 5 5-5",
  bolt: "M13 3.5 5.5 13.5H12l-1 7 7.5-10H12z",
  layout: "M4.5 4.5h15v15h-15zM4.5 10h15M10 10v9.5",
  updown: "M8 9.5l4-4 4 4M8 14.5l4 4 4-4",
  pin: "M9 4.5h6M10 4.5v5L7 13v1.5h10V13l-3-3.5v-5M12 14.5V20",
  follow: "M4.5 12h11M11.5 7.5 16 12l-4.5 4.5M19.5 5v14",
  overlap: "M9.5 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10ZM14.5 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10Z",
  alert: "M12 4.5 20.5 19.5h-17zM12 10v4M12 16.8h.01",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  folder: "M4.5 7V18a1.5 1.5 0 0 0 1.5 1.5h12a1.5 1.5 0 0 0 1.5-1.5V9.5A1.5 1.5 0 0 0 18 8h-6.5L9.5 5.5H6A1.5 1.5 0 0 0 4.5 7Z",
  refresh: "M5 11a7 7 0 0 1 12.5-3.5l2 2M19.5 5v4.5H15M19 13a7 7 0 0 1-12.5 3.5l-2-2M4.5 19v-4.5H9",
  goon: "M5 7h14M5 12h9M5 17h4M13 17h7M17 14l3 3-3 3",
  picture: "M4.5 5.5h15v13h-15zM4.5 15.5l4.5-4.5 4 4 2.5-2.5 4 4M15 9.5h.01",
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, small }: { name: IconName; small?: boolean }) {
  return (
    <svg className={small ? "ic sm" : "ic"} viewBox="0 0 24 24" aria-hidden="true">
      <path d={PATHS[name]} />
    </svg>
  );
}

// The drawing of design/icon/lorelamp-icon-small.svg (256 units): the flame, and the two pages of the
// open book in its belly.
const MARK_FLAME = "M148 32C148 70 186 100 186 158A58 58 0 0 1 70 158C70 124 92 104 90 72C96 92 108 104 118 108C112 80 118 54 148 32Z";
const MARK_PAGES =
  "M123.5 161C114 152 102 148 91.5 149.5Q89 150 89 152.5L90.4 181Q90.6 184 93.5 184C104 183.5 115 187 123.5 194Z" +
  "M132.5 161C142 152 154 148 164.5 149.5Q167 150 167 152.5L165.6 181Q165.4 184 162.5 184C152 183.5 141 187 132.5 194Z";

/** Lorelamp's mark, the app icon's cut for small sizes: a flame with an open book in it on the indigo
 *  tile, in the icon's own colours in every theme (tokens --mark-*). The flame flickers (.mark-flick in
 *  style.css, transform only, still with reduced motion); the book does not move. */
export function BrandMark({ className }: { className: string }) {
  const id = useId();
  return (
    <svg className={className} viewBox="0 0 256 256" aria-hidden="true">
      <defs>
        <linearGradient id={`${id}t`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" style={{ stopColor: "var(--mark-tile-top)" }} />
          <stop offset="1" style={{ stopColor: "var(--mark-tile-bottom)" }} />
        </linearGradient>
        <radialGradient id={`${id}f`} gradientUnits="userSpaceOnUse" cx="128" cy="176" r="150">
          <stop offset="0" style={{ stopColor: "var(--mark-flame-core)" }} />
          <stop offset=".38" style={{ stopColor: "var(--mark-flame-body)" }} />
          <stop offset=".72" style={{ stopColor: "var(--mark-flame-edge)" }} />
          <stop offset="1" style={{ stopColor: "var(--mark-flame-tip)" }} />
        </radialGradient>
      </defs>
      <rect width="256" height="256" rx="58" fill={`url(#${id}t)`} />
      <g transform="translate(128 128) scale(1.14) translate(-128 -126)">
        <path className="mark-flick" d={MARK_FLAME} fill={`url(#${id}f)`} />
        <path d={MARK_PAGES} transform="translate(128 172) scale(1.16) translate(-128 -172)" fill="var(--mark-page)" />
      </g>
    </svg>
  );
}

/** The card's (or persona's) own picture, or the initial on a flat tone when there is none. */
// A character without a picture keeps one colour of its own, so it is recognised at a glance.
// The hues sit at least 30 degrees apart and leave out violet.
const HUES = [14, 38, 68, 128, 162, 192, 214, 332, 352];
export const hueOf = (name: string) => {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.codePointAt(0)!) >>> 0;
  return HUES[h % HUES.length];
};

/** The character's own hue as a CSS variable; the stylesheet derives its ink, tint and field from it. */
export const hueStyle = (name: string) => ({ "--h": hueOf(name) }) as CSSProperties;

type PortraitProps = {
  /** Anything that carries a character's picture: a library entry, a recent chat. */
  who: { id: number; name: string; has_avatar: boolean; avatar_rev: number; avatar_pos: [number, number] };
  className: string;
  /** A picture picked but not saved yet (object URL); shown instead of the stored one. */
  preview?: string;
  /** A crop position chosen but not saved yet. */
  pos?: [number, number];
  /** Whose picture: a character's (default) or a persona's. */
  kind?: "character" | "persona";
  /** A character's original picture instead of its small thumbnail (the editor, the character's own page). */
  full?: boolean;
};

export function Portrait({ who, className, preview, pos, kind = "character", full }: PortraitProps) {
  const [x, y] = pos ?? who.avatar_pos;
  // Grids and lists take the thumbnail (same aspect, so the crop position fits it too).
  const url = kind === "persona" ? personaAvatarUrl : full ? avatarUrl : thumbUrl;
  const src = preview ?? (who.has_avatar ? url(who.id, who.avatar_rev) : null);
  const [broken, setBroken] = useState<string | null>(null);
  const name = who.name;
  if (src && broken !== src) {
    return (
      <img
        className={className}
        src={src}
        alt=""
        loading="lazy"
        decoding="async"
        draggable={false}
        style={{ objectPosition: `${x}% ${y}%` }}
        onError={() => setBroken(src)}
      />
    );
  }
  return (
    <span className={`${className} initial`} style={hueStyle(name)} aria-hidden="true">
      {(name.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}
