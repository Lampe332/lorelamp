// The tag filter of the characters tab: the card's tags and the user's own ones together (case does
// not matter, the same tag in two spellings is one), each with how many cards carry it. Several tags
// narrow the list together. The most used ones sit in a row; all of them, with a search, unfold below.

import { useId, useRef, useState } from "react";
import { num } from "../api";
import { t } from "../i18n";
import { fold } from "../search";
import { Icon } from "../ui";

export type TagCount = { key: string; name: string; count: number };

/** How many chips the row shows besides the chosen ones. */
const ROW = 8;

type Props = {
  tags: TagCount[];
  /** Keys (folded names) of the chosen tags. */
  chosen: string[];
  onChange: (chosen: string[]) => void;
};

export function TagFilter({ tags, chosen, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const [find, setFind] = useState("");
  const panel = useId();
  const moreRef = useRef<HTMLButtonElement>(null);

  const isOn = (key: string) => chosen.includes(key);
  const flip = (key: string) => onChange(isOn(key) ? chosen.filter((k) => k !== key) : [...chosen, key]);
  // The chosen ones first, then the most used ones.
  const row = [...tags.filter((t) => isOn(t.key)), ...tags.filter((t) => !isOn(t.key)).slice(0, ROW)];
  const hidden = tags.length - row.length;
  const f = fold(find.trim()).text;
  const listed = f ? tags.filter((t) => fold(t.name).text.includes(f)) : tags;

  const close = () => {
    setOpen(false);
    setFind("");
    moreRef.current?.focus();
  };

  const chip = (tag: TagCount) => {
    const on = isOn(tag.key);
    return (
      <button key={tag.key} type="button" className={`tag-chip${on ? " on" : ""}`} aria-pressed={on} onClick={() => flip(tag.key)} data-log-label="Tag-Filter">
        {on && <Icon name="check" small />}
        {tag.name}
        <span className="num">{num(tag.count)}</span>
      </button>
    );
  };

  return (
    <div className="lib-tagbar" role="group" aria-label={t("cards.tags.filter")}>
      <div className="tag-filter lib-tagrow">
        <button type="button" className={`tag-chip${chosen.length === 0 ? " on" : ""}`} aria-pressed={chosen.length === 0} onClick={() => onChange([])}>
          {t("cards.tags.all")}
        </button>
        {row.map(chip)}
        {(hidden > 0 || open) && (
          <button
            ref={moreRef}
            type="button"
            className={`tag-chip lib-tagmore${open ? " on" : ""}`}
            aria-expanded={open}
            aria-controls={panel}
            onClick={() => (open ? close() : setOpen(true))}
          >
            {open ? t("cards.tags.less") : t("cards.tags.more", { n: hidden })}
            <span className={open ? "flip" : undefined}>
              <Icon name="chevron" small />
            </span>
          </button>
        )}
      </div>
      {open && (
        <div className="lib-tagpanel" id={panel} onKeyDown={(e) => e.key === "Escape" && close()}>
          <label className="search lib-tagfind">
            <Icon name="search" small />
            <input className="input" autoFocus value={find} onChange={(e) => setFind(e.target.value)} placeholder={t("cards.tags.find")} aria-label={t("cards.tags.find")} />
          </label>
          {listed.length > 0 ? <div className="tag-filter lib-taglist">{listed.map(chip)}</div> : <p className="empty-line">{t("cards.tags.none", { q: find.trim() })}</p>}
        </div>
      )}
    </div>
  );
}
