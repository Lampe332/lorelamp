// The user's own tags of a character (field "Tags" in the card editor): they order the library and
// stay in the app, apart from the card's own tags. Every change is saved at once.

import { useEffect, useId, useState } from "react";
import { api, errorText, postJson, type CharInfo, type TagCount } from "./api";
import { t } from "./i18n";
import { Icon } from "./ui";

const TAG_MAX = 40;

export function OwnTags({ character }: { character: CharInfo }) {
  const [tags, setTags] = useState(character.own_tags);
  const [text, setText] = useState("");
  const [known, setKnown] = useState<TagCount[]>([]);
  const [error, setError] = useState("");
  const listId = useId();

  // The library's state wins whenever it changes (another device, a bulk action).
  const fromLibrary = character.own_tags.join("\u001f");
  useEffect(() => setTags(character.own_tags), [fromLibrary]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    let live = true;
    api<{ tags: TagCount[] }>("/tags")
      .then((r) => live && setKnown(r.tags))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [fromLibrary]);

  const save = async (next: string[]) => {
    const before = tags;
    setTags(next);
    setError("");
    try {
      const r = await postJson<{ tags: string[] }>(`/characters/${character.id}/tags`, { tags: next }, "PUT");
      setTags(r.tags);
    } catch (e) {
      setTags(before);
      setError(errorText(e));
    }
  };

  const add = () => {
    const parts = text
      .split(",")
      .map((t) => t.replace(/\s+/g, " ").trim())
      .filter(Boolean);
    setText("");
    const next = [...tags];
    for (const p of parts) if (!next.some((t) => t.toLocaleLowerCase("de") === p.toLocaleLowerCase("de"))) next.push(p);
    if (next.length !== tags.length) void save(next);
  };

  return (
    <div className="field own-tags" data-spot="own-tags">
      <label className="field-label" htmlFor={`${listId}-in`}>
        {t("cards.tags.own")}
      </label>
      <span className="help">{t("cards.tags.own.help")}</span>
      <div className="tag-edit">
        {tags.map((tag) => (
          <span className="tag-chip on" key={tag}>
            {tag}
            <button type="button" className="tag-x" aria-label={t("cards.tags.own.remove", { name: tag })} data-log-label="Tag entfernen" onClick={() => void save(tags.filter((x) => x !== tag))}>
              <Icon name="close" small />
            </button>
          </span>
        ))}
        <input
          id={`${listId}-in`}
          className="tag-input"
          value={text}
          maxLength={TAG_MAX}
          list={listId}
          placeholder={tags.length ? t("cards.tags.own.next") : t("cards.tag.placeholder")}
          enterKeyHint="done"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === ",") {
              e.preventDefault();
              add();
            } else if (e.key === "Backspace" && text === "" && tags.length > 0) {
              void save(tags.slice(0, -1));
            }
          }}
          onBlur={() => text.trim() && add()}
        />
        <datalist id={listId}>
          {known
            .filter((k) => !tags.some((t) => t.toLocaleLowerCase("de") === k.name.toLocaleLowerCase("de")))
            .map((k) => (
              <option key={k.name} value={k.name} />
            ))}
        </datalist>
      </div>
      {error && (
        <span className="help bad" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}
