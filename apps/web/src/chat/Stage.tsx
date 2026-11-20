// Two pictures of a chat: the character shown large next to the story (a side area while the chat
// has room for both, a band above the story in a narrow one), and the backdrop behind the text.

import { useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { avatarUrl, type CharInfo } from "../api";
import { t } from "../i18n";
import { Icon, Portrait } from "../ui";
import type { Frame } from "./look";

/** How much of the picture filling the area may cut away (share of its width or height) before
 *  the whole picture is shown instead. */
const MAX_CUT = 0.3;

/** How a picture of shape `pic` (width / height) sits in an area of shape `area`. */
export function stageFit(pic: number, area: number): "cover" | "contain" {
  const cut = 1 - Math.min(pic / area, area / pic);
  return cut <= MAX_CUT ? "cover" : "contain";
}

/** The character's own picture, large. It stays while you read and write. */
export function Stage({ who, onClose }: { who: CharInfo; onClose: () => void }) {
  const box = useRef<HTMLElement>(null);
  // "cover" while the area has about the picture's shape (a little is cut at two edges); otherwise
  // the whole picture is shown, and the same picture, soft and dim, fills what it leaves free.
  const [fit, setFit] = useState<"cover" | "contain">("cover");
  const read = () => {
    const el = box.current;
    const img = el?.querySelector("img");
    if (!el || !img || !img.naturalWidth || !el.clientWidth || !el.clientHeight) return;
    setFit(stageFit(img.naturalWidth / img.naturalHeight, el.clientWidth / el.clientHeight));
  };
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    read();
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const fill = who.has_avatar ? ({ "--cv-pic": `url("${avatarUrl(who.id, who.avatar_rev)}")` } as CSSProperties) : undefined;
  return (
    <aside className="cv-stage" aria-label={t("chat.stage.label", { name: who.name })} style={fill} data-fit={fit} ref={box} onLoadCapture={read}>
      <Portrait who={who} className="cv-stage-pic" full />
      <button type="button" className="icon-btn cv-stage-close" onClick={onClose} aria-label={t("chat.bar.stageHide")} title={t("chat.bar.stageHide")}>
        <Icon name="close" small />
      </button>
    </aside>
  );
}

/** The picture behind the story, under a veil of the theme's ground so the text stays readable. */
export function Backdrop({ url, frame }: { url: string; frame: Frame }) {
  // A picture that cannot be loaded (its file is gone) leaves the plain ground.
  const [broken, setBroken] = useState<string | null>(null);
  if (broken === url) return null;
  return (
    <div className="cv-backdrop" aria-hidden="true" data-blur={frame.blur} data-dim={frame.dim}>
      <img
        src={url}
        alt=""
        draggable={false}
        decoding="async"
        style={{ objectPosition: `${frame.x}% ${frame.y}%`, filter: frame.blur ? `blur(${frame.blur}px)` : undefined, transform: frame.blur ? `scale(${1 + frame.blur / 200})` : undefined }}
        onError={() => setBroken(url)}
      />
      <i style={{ opacity: frame.dim / 100 }} />
    </div>
  );
}
