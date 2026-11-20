// The gallery of ready-made backdrops inside "Aussehen": small pictures to pick from, for this chat
// or as the default of every chat with the character, plus "Kein Hintergrund". The picture the
// chosen target shows now is marked.

import { useState } from "react";
import { t } from "../i18n";
import { Icon } from "../ui";
import { deleteCharacterBackdrop, putChatLook, type ChatLook } from "./look";
import { PRESETS, presetName, presetThumbUrl, putCharacterPreset, putChatPreset } from "./presets";
import "./presets.css";

type Props = {
  chatId: number;
  name: string;
  view: ChatLook;
  /** What the chat shows now (its own picture, the character's, or none). */
  shownPreset: string | undefined;
  showsNone: boolean;
  busy: boolean;
  run: (work: () => Promise<unknown>, done?: string) => Promise<void>;
};

export function PresetGallery({ chatId, name, view, shownPreset, showsNone, busy, run }: Props) {
  const [target, setTarget] = useState<"chat" | "character">("chat");
  const [pending, setPending] = useState<string | null>(null);
  const theirs = view.backdrop.character;
  const forChat = target === "chat";
  const active = forChat ? shownPreset : theirs?.preset;
  const noneActive = forChat ? showsNone : !theirs;

  const pick = (id: string | null) => {
    if (busy || (id === null ? noneActive : id === active)) return;
    setPending(id ?? "none");
    const label = id ? presetName(id) : undefined;
    let work: () => Promise<unknown>;
    let done = "";
    if (forChat) {
      work = () => (id ? putChatPreset(chatId, id) : putChatLook(chatId, { backdrop: "none" }));
    } else if (id) {
      // The character's new default shows here at once, also in place of this chat's own picture
      // (that one stays and can be picked again under "Eigenes").
      work = async () => {
        await putCharacterPreset(view.character_id, id);
        if (view.backdrop.mode !== "character") await putChatLook(chatId, { backdrop: "character" });
      };
      done = t("chat.presets.doneChar", { name, label: label ?? id });
    } else {
      work = () => deleteCharacterBackdrop(view.character_id);
      done = t("chat.presets.doneNone", { name });
    }
    void run(work, done).finally(() => setPending(null));
  };

  return (
    <div className="bp" role="group" aria-label={t("chat.presets")}>
      <div className="bp-head">
        <h4>{t("chat.presets")}</h4>
        <div className="seg cv-seg" role="group" aria-label={t("chat.presets.for")}>
          <button type="button" aria-pressed={forChat} onClick={() => setTarget("chat")}>
            {t("chat.presets.thisChat")}
          </button>
          <button type="button" aria-pressed={!forChat} onClick={() => setTarget("character")}>
            {t("chat.presets.allChats", { name })}
          </button>
        </div>
      </div>
      <ul className="bp-grid" aria-label={forChat ? t("chat.presets.gridChat") : t("chat.presets.gridChar", { name })}>
        <li>
          <button type="button" className="bp-tile bp-none" aria-pressed={noneActive} data-pending={pending === "none" || undefined} disabled={busy && pending !== "none"} onClick={() => pick(null)}>
            <span className="bp-name">{t("chat.presets.none")}</span>
            {noneActive && <Mark />}
          </button>
        </li>
        {PRESETS.map((p) => (
          <li key={p.id}>
            <button
              type="button"
              className="bp-tile"
              aria-label={p.name}
              title={p.name}
              aria-pressed={active === p.id}
              data-pending={pending === p.id || undefined}
              disabled={busy && pending !== p.id}
              onClick={() => pick(p.id)}
            >
              <img src={presetThumbUrl(p.id)} alt="" width={400} height={225} loading="lazy" decoding="async" draggable={false} />
              <span className="bp-name">{p.name}</span>
              {active === p.id && <Mark />}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Mark() {
  return (
    <span className="bp-mark" aria-hidden="true">
      <Icon name="check" small />
    </span>
  );
}
