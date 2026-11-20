// The card builder (`#/chars/new`, `#/chars/new/from/<id>`): plan a new character, build its card
// (by hand, or field by field with the local model), see it as the library and the chat will, then
// save it into the library through the same path as an imported card, chat with it and export it.
// The draft is kept on this device until it is saved or thrown away.

import { useEffect, useRef, useState } from "react";
import { api, ApiError, avatarUrl, errorText, postJson, type CharInfo, type Library } from "../api";
import { useFormFactor } from "../app/formfactor";
import { CardExportDialog } from "../CardExport";
import { t } from "../i18n";
import type { Picked } from "../image";
import { Icon, Portrait } from "../ui";
import "./builder.css";
import { applyOutline, draftOf, DRAFT_ORDER, emptyDraft, emptyPlan, filled, isBlank, saveDraft, sizes, writeField, type Draft, type Field, type Plan } from "./draft";
import { Fields, type Writing } from "./Fields";
import { dropKept, dropPicture, loadKept, loadPicture, storeKept, storePicture, type Step } from "./keep";
import { PlanStep } from "./PlanStep";
import { Preview } from "./Preview";

type Props = {
  library: Library | null;
  /** `#/chars/new/from/<id>`: start from a copy of this character. */
  from: number | null;
  onToast: (text: string) => void;
};

const STEPS: Step[] = ["plan", "build", "preview"];

/** What the model's answer does to the draft. */
function applyAnswer(d: Draft, field: Field, value: string | string[]): Draft {
  if (field === "tags") return { ...d, tags: Array.isArray(value) ? value : [value] };
  const text = Array.isArray(value) ? value.join(", ") : value;
  if (field === "alt_greeting") return { ...d, alternate_greetings: [...d.alternate_greetings.filter((g) => g.trim()), text] };
  return { ...d, [field]: text };
}

/** The words for a draft that did not work: the model's state in plain terms. */
function draftNote(e: unknown): { text: string; model?: boolean } {
  const code = e instanceof ApiError ? e.code : "";
  if (code === "no_model") return { text: t("builder.note.noModel"), model: true };
  if (code === "draft_empty") return { text: t("builder.note.empty") };
  if (code === "draft_failed") return { text: t("builder.note.failed") };
  return { text: t("builder.note.error", { error: errorText(e) }) };
}

export function Builder({ library, from, onToast }: Props) {
  const form = useFormFactor();
  const [kept] = useState(loadKept);
  const [plan, setPlan] = useState<Plan>(kept.plan);
  const [draft, setDraft] = useState<Draft>(kept.draft);
  const [source, setSource] = useState<number | null>(kept.from);
  const [step, setStep] = useState<Step>(kept.step);
  const [picked, setPicked] = useState<Picked | null>(null);
  const [pos, setPos] = useState<[number, number]>([50, 50]);
  const [writing, setWriting] = useState<Writing>(null);
  const [note, setNote] = useState<{ text: string; model?: boolean } | null>(null);
  const [undo, setUndo] = useState<Partial<Record<Field, Draft>>>({});
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState<CharInfo | null>(null);
  const [exporting, setExporting] = useState(false);
  const [confirmDrop, setConfirmDrop] = useState(false);
  // A copy waits for a yes when it would replace a draft with text in it.
  const [askTemplate, setAskTemplate] = useState<number | null>(null);
  const stop = useRef<AbortController | null>(null);
  // The newest draft for the model's requests, which run across several renders.
  const latest = useRef(draft);
  const top = useRef<HTMLDivElement>(null);

  useEffect(() => {
    latest.current = draft;
  }, [draft]);

  // Every change is kept on this device at once.
  useEffect(() => {
    if (!done) storeKept({ plan, draft, from: source, step });
  }, [plan, draft, source, step, done]);

  // The picked picture comes back after a reload.
  useEffect(() => {
    let stale = false;
    void loadPicture().then((p) => {
      if (stale || !p) return;
      const file = new File([p.blob], p.name, { type: p.blob.type });
      setPicked({ file, url: URL.createObjectURL(file), pos: p.pos });
      setPos(p.pos);
    });
    return () => {
      stale = true;
    };
  }, []);
  useEffect(() => () => void (picked && URL.revokeObjectURL(picked.url)), [picked]);
  useEffect(() => () => stop.current?.abort(), []);

  const pick = (p: Picked | null) => {
    setPicked(p);
    if (p) {
      setPos(p.pos);
      void storePicture({ blob: p.file, name: p.file.name, pos: p.pos });
    } else {
      void dropPicture();
    }
  };
  const movePos = (next: [number, number]) => {
    setPos(next);
    if (picked) void storePicture({ blob: picked.file, name: picked.file.name, pos: next });
  };

  const editPlan = (patch: Partial<Plan>) => setPlan((p) => ({ ...p, ...patch }));
  const editDraft = (patch: Partial<Draft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setError("");
  };
  const go = (next: Step) => {
    setStep(next);
    top.current?.scrollIntoView({ block: "start" });
  };

  // ---- a copy of an existing character as the start

  const takeTemplate = async (id: number) => {
    setAskTemplate(null);
    try {
      const { draft: copy } = await draftOf(id);
      const info = library?.characters.find((c) => c.id === id);
      setDraft({ ...copy, name: t("builder.template.copyName", { name: copy.name }) });
      setPlan((p) => ({ ...p, idea: p.idea || copy.creator_notes.split("\n")[0] || "" }));
      setSource(id);
      setUndo({});
      setStep("build");
      if (info?.has_avatar) {
        const res = await fetch(avatarUrl(id, info.avatar_rev));
        if (res.ok) {
          const blob = await res.blob();
          const file = new File([blob], `${id}.${blob.type.split("/")[1] ?? "png"}`, { type: blob.type });
          pick({ file, url: URL.createObjectURL(file), pos: info.avatar_pos });
        }
      } else {
        pick(null);
      }
      onToast(t("builder.template.done", { name: copy.name }));
    } catch (e) {
      onToast(t("builder.template.failed", { error: errorText(e) }));
    }
  };
  const askForTemplate = (id: number) => {
    if (isBlank(plan, latest.current) && !picked) void takeTemplate(id);
    else setAskTemplate(id);
  };
  // `#/chars/new/from/<id>`: once, then the address is plain again (a reload does not copy again).
  useEffect(() => {
    if (from === null) return;
    history.replaceState(null, "", "#/chars/new");
    askForTemplate(from);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [from]);

  // ---- the local model

  const request = async (field: Field, all: boolean): Promise<"ok" | "stopped" | "failed"> => {
    const ctrl = new AbortController();
    stop.current = ctrl;
    setWriting({ field, all });
    setNote(null);
    try {
      const r = await writeField({ plan, draft: latest.current, field, from: source }, ctrl.signal);
      const before = latest.current;
      if (filled(before, field)) setUndo((u) => ({ ...u, [field]: before }));
      // The next field of "Entwurf schreiben lassen" already sees this one.
      latest.current = applyAnswer(before, field, r.value);
      setDraft((d) => applyAnswer(d, field, r.value));
      return "ok";
    } catch (e) {
      if (ctrl.signal.aborted) return "stopped";
      setNote(draftNote(e));
      return "failed";
    } finally {
      if (stop.current === ctrl) stop.current = null;
      setWriting(null);
    }
  };
  const writeOne = (field: Field) => void request(field, false);
  const writeAll = async () => {
    for (const field of DRAFT_ORDER) {
      if (filled(latest.current, field)) continue;
      if ((await request(field, true)) !== "ok") return;
    }
    onToast(t("builder.write.allDone"));
  };
  const stopWriting = () => stop.current?.abort();
  const takeBack = (field: Field) => {
    const before = undo[field];
    if (!before) return;
    // Only the field goes back; everything else stays as it is now.
    setDraft((d) => (field === "alt_greeting" ? { ...d, alternate_greetings: before.alternate_greetings } : { ...d, [field]: before[field] }));
    setUndo((u) => {
      const next = { ...u };
      delete next[field];
      return next;
    });
  };
  const emptyCount = DRAFT_ORDER.filter((f) => !filled(draft, f)).length;

  // ---- saving

  const reset = () => {
    stop.current?.abort();
    setPlan(emptyPlan());
    setDraft(emptyDraft());
    setSource(null);
    setStep("plan");
    setUndo({});
    setNote(null);
    setError("");
    pick(null);
    dropKept();
  };
  const save = async () => {
    if (!draft.name.trim()) {
      setError(t("builder.save.noName"));
      go("build");
      return;
    }
    setSaving(true);
    setError("");
    try {
      let info = await saveDraft(draft, source);
      if (picked) {
        await api(`/characters/${info.id}/avatar`, { method: "PUT", body: picked.file });
        await postJson(`/characters/${info.id}/avatar-position`, { x: pos[0], y: pos[1] }, "PUT");
        info = { ...info, has_avatar: true, avatar_pos: pos, avatar_rev: info.avatar_rev + 1 };
      }
      dropKept();
      void dropPicture();
      setDone(info);
      top.current?.scrollIntoView({ block: "start" });
    } catch (e) {
      setError(t("builder.save.failed", { error: errorText(e) }));
    } finally {
      setSaving(false);
    }
  };
  const startChat = async (id: number) => {
    try {
      const { chat_id } = await postJson<{ chat_id: number }>("/chats", { character_id: id, fresh: true });
      location.hash = `#/chat/${chat_id}`;
    } catch (e) {
      onToast(t("builder.done.chatFailed", { error: errorText(e) }));
    }
  };
  const again = () => {
    setDone(null);
    reset();
  };

  const userName = library?.personas.find((p) => p.id === library.active_persona_id)?.name ?? library?.user_name ?? "User";
  const total = sizes(draft);

  if (done) {
    return (
      <div className="view">
        <div className="page cb-page" ref={top}>
          <section className="cb-done" aria-live="polite">
            <Portrait who={done} className="cb-done-pic" full />
            <div className="cb-done-text">
              <h1>{t("builder.done.title", { name: done.name })}</h1>
              <p>{t("builder.done.text")}</p>
              <div className="btn-row">
                <button type="button" className="btn primary" autoFocus onClick={() => void startChat(done.id)}>
                  <Icon name="chat" small />
                  {t("builder.done.chat")}
                </button>
                <button type="button" className="btn" aria-haspopup="dialog" onClick={() => setExporting(true)}>
                  <Icon name="download" small />
                  {t("builder.done.export")}
                </button>
                <a className="btn" href={`#/char/${done.id}`}>
                  <Icon name="card" small />
                  {t("builder.done.edit")}
                </a>
                <button type="button" className="btn ghost" onClick={again}>
                  <Icon name="plus" small />
                  {t("builder.done.again")}
                </button>
              </div>
            </div>
          </section>
          {exporting && <CardExportDialog characterId={done.id} name={done.name} hasPicture={done.has_avatar} onClose={() => setExporting(false)} onSaved={(file) => onToast(t("builder.done.exported", { file }))} />}
        </div>
      </div>
    );
  }

  const side = form === "desktop" && step !== "preview";
  // "Weiter" leads until the draft has a name and a description; from then on (and on the preview) saving does.
  const savable = step === "preview" || (!!draft.name.trim() && !!draft.description.trim());
  return (
    <div className="view">
      <div className="page cb-page" ref={top}>
        <div className="title-row cb-title">
          <a className="btn icon-sm ghost" href="#/chars" aria-label={t("builder.back")} title={t("builder.back")}>
            <Icon name="back" small />
          </a>
          <h1>{t("builder.title")}</h1>
        </div>

        <nav className="cb-steps" aria-label={t("builder.steps")}>
          <ol>
            {STEPS.map((s, i) => (
              <li key={s}>
                <button type="button" aria-current={step === s ? "step" : undefined} aria-label={t(`builder.step.${s}`)} onClick={() => go(s)}>
                  <span className="cb-step-n num">{i + 1}</span>
                  <span className="cb-step-l">{t(`builder.step.${s}`)}</span>
                </button>
              </li>
            ))}
          </ol>
        </nav>

        {askTemplate !== null && (
          <div className="well cb-ask" role="alertdialog" aria-label={t("builder.template.ask")}>
            <p>{t("builder.template.ask")}</p>
            <div className="btn-row">
              <button type="button" className="btn primary" onClick={() => void takeTemplate(askTemplate)}>
                {t("builder.template.replace")}
              </button>
              <button type="button" className="btn ghost" onClick={() => setAskTemplate(null)}>
                {t("builder.template.keep")}
              </button>
            </div>
          </div>
        )}

        <div className={`cb-body${side ? " with-side" : ""}`}>
          <div className="cb-main set">
            {step === "plan" && (
              <PlanStep
                plan={plan}
                onPlan={editPlan}
                characters={library?.characters ?? null}
                onTemplate={askForTemplate}
                onUseOutline={() => {
                  setDraft((d) => applyOutline(plan, d));
                  go("build");
                }}
              />
            )}
            {step === "build" && (
              <>
                <div className="well cb-assist">
                  <div>
                    <b>{t("builder.assist.title")}</b>
                    <p>{t("builder.assist.text")}</p>
                    {note && (
                      <p className="cb-note" role="alert">
                        {note.text}{" "}
                        {note.model && (
                          <a href="#/settings/model" className="cb-link">
                            {t("builder.note.toModel")}
                          </a>
                        )}
                      </p>
                    )}
                  </div>
                  {writing?.all ? (
                    <button type="button" className="btn" onClick={stopWriting}>
                      {t("builder.write.stopAll")}
                    </button>
                  ) : (
                    <button type="button" className="btn solid" disabled={writing !== null || emptyCount === 0} onClick={() => void writeAll()}>
                      <Icon name="pencil" small />
                      {emptyCount === 0 ? t("builder.write.allFull") : t("builder.write.all", { n: emptyCount })}
                    </button>
                  )}
                </div>
                <Fields draft={draft} onDraft={editDraft} writing={writing} onWrite={writeOne} onStop={stopWriting} undo={undo} onUndo={takeBack} picked={picked} pos={pos} onPick={pick} onPos={movePos} />
              </>
            )}
            {step === "preview" && <Preview draft={draft} picked={picked} pos={pos} userName={userName} full />}
          </div>
          {side && (
            <aside className="cb-side">
              <Preview draft={draft} picked={picked} pos={pos} userName={userName} full={false} />
            </aside>
          )}
        </div>

        <div className={`savebar cb-bar${isBlank(plan, draft) ? "" : " dirty"}`}>
          <span className={error ? "bad" : confirmDrop ? "cb-asking" : "muted"} role="status">
            {error || (confirmDrop ? t("builder.drop.ask") : t("builder.kept", { n: total.always + total.greeting }))}
          </span>
          {confirmDrop ? (
            <>
              <button type="button" className="btn ghost" onClick={() => setConfirmDrop(false)}>
                {t("builder.drop.keep")}
              </button>
              <button
                type="button"
                className="btn danger"
                onClick={() => {
                  setConfirmDrop(false);
                  reset();
                }}
              >
                {t("builder.drop.yes")}
              </button>
            </>
          ) : (
            <>
              <button type="button" className="btn ghost cb-drop" disabled={isBlank(plan, draft) && !picked} aria-label={t("builder.drop")} title={t("builder.drop")} onClick={() => setConfirmDrop(true)}>
                <Icon name="trash" small />
                <span className="hide-sm">{t("builder.drop")}</span>
              </button>
              {step !== "preview" && (
                <button type="button" className={`btn${savable ? "" : " primary"}`} aria-label={t(step === "plan" ? "builder.next.build" : "builder.next.preview")} onClick={() => go(STEPS[STEPS.indexOf(step) + 1])}>
                  <span className="hide-sm">{t(step === "plan" ? "builder.next.build" : "builder.next.preview")}</span>
                  <span className="show-sm">{t("builder.next.short")}</span>
                  <Icon name="next" small />
                </button>
              )}
              <button type="button" className={`btn${savable ? " primary" : ""}`} disabled={saving || writing !== null} aria-label={t("builder.save")} onClick={() => void save()}>
                <span className="hide-sm">{saving ? t("builder.save.saving") : t("builder.save")}</span>
                <span className="show-sm">{saving ? t("builder.save.saving") : t("builder.save.short")}</span>
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

