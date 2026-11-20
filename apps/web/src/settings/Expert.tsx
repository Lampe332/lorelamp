// Settings tab "Experte": everything about a model profile, exactly. The profiles (activate, edit,
// duplicate, rename, delete, new), then the edited profile: its model file and engine, every sampler
// value ("Antwort", with slider, field, one line and "Standard"), thinking, and the start options
// ("Start": context with its memory, GPU layers, MoE layers on the CPU, KV cache, flash attention,
// threads, batch sizes, mmap/mlock and every other argument as text). Changes wait for "Speichern";
// start options of the active profile restart the model. Below: the model server's port, the model
// folders, the PC scan and the engines.

import { useEffect, useId, useState, type ReactNode } from "react";
import { ApiError, ctxShort, engineName, errorText, gb, num, postJson, type Engine, type EngineInfo, type Estimate, type LocalModel, type MemoryFit as Fit, type Profile, type Sampler } from "../api";
import { t, texts } from "../i18n";
import { rich } from "../i18n/rich";
import { Icon } from "../ui";
import { PortField } from "./EngineNote";
import { buildArgs, draftOf, KEYS, managedIn, parseRest, restText, splitArgs, type LoadDraft, type LoadKey } from "./args";
import { badValue, decimals, GROUPS, KNOBS, type Knob } from "./knobs";
import { ModelFolderList } from "./LocalModels";
import { PcOnly } from "../phone/Phone";
import { onOtherDevice } from "../phone/shell";
import { PcScan } from "./PcScan";
import { ModelOptions, ProfileList } from "./Profiles";
import { SwitchProgress, type Progress } from "./switch";
import { useModels } from "./useModels";

const FIT_TEXT: Record<Exclude<Fit, "unknown">, string> = texts({}, { fits: "models.fit.fits", cpu_help: "models.fit.cpu_help", too_big: "models.fit.too_big" });
const FIT_NOTE: Record<Exclude<Fit, "unknown">, string> = texts({}, { fits: "models.ex.fit.fits", cpu_help: "models.ex.fit.cpu_help", too_big: "models.ex.fit.too_big" });
const KV_TYPES = ["f16", "q8_0", "q4_0", "bf16", "f32", "q5_1", "q5_0", "q4_1", "iq4_nl"];
const BACKEND: Record<string, string> = { cpu: "CPU", rocm: "ROCm", cuda: "CUDA", vulkan: "Vulkan", metal: "Metal", sycl: "SYCL" };

export function Expert() {
  const m = useModels();
  const [chosen, setChosen] = useState<number | null>(null);
  /** The editor holds changes that are not saved: another profile waits until they are. */
  const [locked, setLocked] = useState(false);
  const p = m.profiles?.find((x) => x.id === chosen) ?? m.active ?? m.profiles?.[0] ?? null;
  const edit = (id: number) => {
    setChosen(id);
    const calm = matchMedia("(prefers-reduced-motion: reduce)").matches;
    requestAnimationFrame(() => document.querySelector('[data-spot="expert-profile"]')?.scrollIntoView({ block: "start", behavior: calm ? "auto" : "smooth" }));
  };
  const loading = (
    <div aria-busy="true">
      <span className="bar-ph" />
      <span className="bar-ph short" />
    </div>
  );
  return (
    <div className="view">
      <div className="page set mx-expert">
        {m.error && <p className="bad">{m.error}</p>}
        <Row title={t("models.profiles")} text={t("models.ex.profiles.note")} spot="profiles">
          {m.profiles === null ? (
            !m.error && loading
          ) : (
            <ProfileList
              profiles={m.profiles}
              engine={m.engine}
              progress={m.progress}
              switching={m.switching}
              editing={p?.id ?? null}
              locked={locked}
              models={m.local?.models ?? null}
              onActivate={(x) => void m.activate(x)}
              onEdit={edit}
              onChanged={m.loadAll}
            />
          )}
        </Row>
        {p && m.profiles && (
          <Editor
            key={p.id}
            p={p}
            profiles={m.profiles}
            engine={m.engine}
            engines={m.engines}
            models={m.local?.models ?? null}
            progress={m.progress?.id === p.id ? m.progress : null}
            onChoose={setChosen}
            onDirty={setLocked}
            onSaved={m.loadAll}
          />
        )}
        <Row title={t("models.ex.port")} text={t("models.ex.port.note")} spot="engine-port">
          <PortField engine={m.engine} onChanged={m.loadEngine} />
        </Row>
        <Row title={t("models.folders")} text={t("models.ex.folders.note")} spot="model-folders">
          <PcOnly what={t("models.folders")}>
            <ModelFolderList folders={m.local?.folders ?? null} onChanged={m.loadAll} />
          </PcOnly>
        </Row>
        <PcScan />
      </div>
    </div>
  );
}

function Row({ title, text, spot, children, panel = true }: { title: string; text: ReactNode; spot?: string; children: ReactNode; panel?: boolean }) {
  return (
    <div className="set-row">
      <div className="set-label">
        <h2>{title}</h2>
        <p>{text}</p>
      </div>
      {panel ? (
        <div className="set-panel" data-spot={spot}>
          {children}
        </div>
      ) : (
        children
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// The editor of one profile

type SamplerText = Record<keyof Sampler, string>;
type Form = { text: SamplerText; reasoning: "off" | "on"; load: LoadDraft; rest: string; file: string; engineId: number };

const formOf = (p: Profile): Form => ({
  text: Object.fromEntries(KNOBS.map((k) => [k.key, p.sampler[k.key] === undefined ? "" : String(p.sampler[k.key])])) as SamplerText,
  reasoning: p.reasoning,
  load: draftOf(p.load_args),
  rest: restText(splitArgs(p.load_args).rest),
  file: p.model_file,
  engineId: p.engine_id,
});

/** What the editor shows of a profile; another name or the measurement does not touch a draft. */
const shown = (p: Profile) => JSON.stringify([p.sampler, p.reasoning, p.load_args, p.model_file, p.engine_id]);

function samplerOf(text: SamplerText): Sampler {
  const out: Sampler = {};
  for (const k of KNOBS) if (text[k.key] !== "") out[k.key] = Number(text[k.key]);
  return out;
}

/** Read in the language on screen each time (`texts`). */
const LOAD_LABEL: Record<LoadKey, string> = texts(
  { fa: "Flash-Attention", mmap: "mmap", mlock: "mlock" },
  {
    ctx: "models.sug.ctx",
    ngl: "models.ex.load.ngl",
    ncmoe: "models.ex.load.ncmoe",
    ctk: "models.ex.load.kv",
    ctv: "models.ex.load.kv",
    threads: "models.ex.load.threads",
    batch: "models.ex.load.batch",
    ub: "models.ex.load.ub",
  },
);

type EditorProps = {
  p: Profile;
  profiles: Profile[];
  engine: EngineInfo | null;
  engines: Engine[];
  models: LocalModel[] | null;
  progress: Progress | null;
  onChoose: (id: number) => void;
  onDirty: (dirty: boolean) => void;
  onSaved: () => Promise<void>;
};

function Editor({ p, profiles, engine, engines, models, progress, onChoose, onDirty, onSaved }: EditorProps) {
  const base = useId();
  /** The profile as it was when the draft started. */
  const [saved, setSaved] = useState(p);
  const [form, setForm] = useState<Form>(() => formOf(p));
  const [state, setState] = useState<"idle" | "saving" | "saved">("idle");
  const [error, setError] = useState("");
  const start = formOf(saved);
  const dirty = JSON.stringify(form) !== JSON.stringify(start);

  useEffect(() => {
    onDirty(dirty);
    return () => onDirty(false);
  }, [dirty, onDirty]);

  // Saved elsewhere (another device, the benchmark, a switch): follow, unless something here waits.
  useEffect(() => {
    if (shown(p) === shown(saved)) return;
    if (!dirty) {
      setSaved(p);
      setForm(formOf(p));
    }
  }, [p, saved, dirty]);

  const put = <K extends keyof Form>(key: K, value: Form[K]) => {
    setForm((f) => ({ ...f, [key]: value }));
    setState("idle");
    setError("");
  };
  const setText = (key: keyof Sampler, v: string) => put("text", { ...form.text, [key]: v });
  const setLoad = (key: LoadKey, v: string | null) => put("load", { ...form.load, [key]: v === "" ? null : v });

  // What changed, for the save bar and the restart note.
  const loadChanged = KEYS.filter((k) => form.load[k] !== start.load[k]);
  const restChanged = form.rest.trim() !== start.rest.trim();
  const launchChanged = loadChanged.length > 0 || restChanged || form.file !== start.file || form.engineId !== start.engineId;
  const samplerChanged = JSON.stringify([form.text, form.reasoning]) !== JSON.stringify([start.text, start.reasoning]);
  const restArgs = parseRest(form.rest);

  // Problems that keep "Speichern" off.
  const bad =
    KNOBS.map((k) => badValue(k, form.text[k.key])).find(Boolean) ??
    START.map((s) => (form.load[s.key] === null || s.ok(form.load[s.key]!) ? null : `${s.label}: ${s.rule}`)).find(Boolean) ??
    (managedIn(restArgs) ? t("models.ex.managed", { flag: managedIn(restArgs)! }) : null);

  const save = async () => {
    setState("saving");
    setError("");
    const body: Record<string, unknown> = {};
    if (samplerChanged) {
      body.sampler = samplerOf(form.text);
      body.reasoning = form.reasoning;
    }
    if (loadChanged.length || restChanged) body.load_args = buildArgs(p.load_args, start.load, form.load, restChanged ? restArgs : null);
    if (form.file !== start.file) body.model_file = form.file;
    if (form.engineId !== start.engineId) body.engine_id = form.engineId;
    try {
      const next = await postJson<Profile>(`/models/profiles/${p.id}`, body, "PUT");
      setSaved(next);
      setForm(formOf(next));
      setState("saved");
      await onSaved();
    } catch (err) {
      setState("idle");
      setError(
        err instanceof ApiError && err.code === "busy" ? t("models.ex.busy") : t("models.notSaved", { error: errorText(err) }),
      );
    }
  };
  const discard = () => {
    setSaved(p);
    setForm(formOf(p));
    setError("");
    setState("idle");
  };

  // The memory estimate for the drafted start options, at every context step.
  const [est, setEst] = useState<Estimate | null>(null);
  const [estError, setEstError] = useState("");
  const intOf = (v: string | null) => (v !== null && /^\d+$/.test(v) ? Number(v) : null);
  const ctxSet = intOf(form.load.ctx) || null;
  const ngl = intOf(form.load.ngl);
  const ncmoe = intOf(form.load.ncmoe);
  const ctk = form.load.ctk ?? "f16";
  const ctv = form.load.ctv ?? "f16";
  useEffect(() => {
    // Reading a model file's header runs at the PC only (403 `pc_only` from another device).
    if (onOtherDevice) return;
    let gone = false;
    const timer = window.setTimeout(async () => {
      try {
        const e = await postJson<Estimate>("/models/estimate", { path: form.file, cache_k: ctk, cache_v: ctv, gpu_layers: ngl ?? undefined, n_cpu_moe: ncmoe ?? undefined, ctx: ctxSet ?? undefined });
        if (!gone) {
          setEst(e);
          setEstError("");
        }
      } catch (err) {
        if (!gone) setEstError(errorText(err));
      }
    }, 200);
    return () => {
      gone = true;
      window.clearTimeout(timer);
    };
  }, [form.file, ctk, ctv, ngl, ncmoe, ctxSet]);

  const layers = est?.layers ?? null;
  const steps = est?.steps ?? [];
  const ctxNow = ctxSet ?? est?.ctx_train ?? (p.active ? engine?.model.ctx : null) ?? null;
  const stepAt = ctxNow === null ? -1 : steps.findIndex((s) => s.ctx >= ctxNow);
  const step = stepAt >= 0 && steps[stepAt].ctx === ctxNow ? steps[stepAt] : null;
  const dropped = p.active ? (engine?.dropped_args ?? []) : [];
  const restartText = p.active ? t("models.ex.restart") : t("models.ex.whenActive");

  return (
    <>
      <Row title={t("models.ex.edit")} text={t("models.ex.edit.note")} spot="expert-profile">
        <div className="mx-pick">
          <div className="field">
            <label className="field-label" htmlFor={`${base}-profile`}>
              {t("models.hero.profile")}
            </label>
            <select id={`${base}-profile`} className="input ms-select" value={p.id} disabled={dirty} onChange={(e) => onChoose(Number(e.target.value))}>
              {profiles.map((x) => (
                <option key={x.id} value={x.id}>
                  {x.active ? t("models.ex.activeOption", { name: x.name }) : x.name}
                </option>
              ))}
            </select>
            {dirty && <span className="help">{t("models.ex.dirty")}</span>}
          </div>
          <div className="field">
            <label className="field-label" htmlFor={`${base}-file`}>
              {t("models.ex.file")} <RestartTag on={form.file !== start.file} />
            </label>
            <select id={`${base}-file`} className="input ms-select" value={form.file} onChange={(e) => put("file", e.target.value)} title={form.file}>
              <ModelOptions models={models} keep={form.file} />
            </select>
          </div>
          <div className="field">
            <label className="field-label" htmlFor={`${base}-engine`}>
              Engine <RestartTag on={form.engineId !== start.engineId} />
            </label>
            <select id={`${base}-engine`} className="input ms-select" value={form.engineId} onChange={(e) => put("engineId", Number(e.target.value))}>
              {!engines.some((x) => x.id === form.engineId) && <option value={form.engineId}>{t("models.engine.row", { name: String(form.engineId) })}</option>}
              {engines.map((x) => (
                <option key={x.id} value={x.id}>
                  {[engineName(x), BACKEND[x.backend], x.exists ? null : t("models.ex.engineMissing")].filter(Boolean).join(" · ")}
                </option>
              ))}
            </select>
          </div>
        </div>
      </Row>

      <Row title={t("models.ex.answer")} text={t("models.ex.answer.note")} spot="sampler">
        {GROUPS.map((g) => (
          <section key={g.id} className="mx-group" aria-labelledby={`${base}-${g.id}`}>
            <div className="mx-group-head">
              <h3 id={`${base}-${g.id}`}>{g.title}</h3>
              <p>{g.note}</p>
            </div>
            <div className="mx-knobs">
              {g.knobs.map((k) => (
                <SamplerKnob key={k.key} k={k} id={`${base}-${k.key}`} text={form.text[k.key]} changed={form.text[k.key] !== start.text[k.key]} onText={(v) => setText(k.key, v)} />
              ))}
            </div>
          </section>
        ))}
        <section className="mx-group" aria-labelledby={`${base}-think`}>
          <div className="mx-group-head">
            <h3 id={`${base}-think`}>{t("models.ex.think")}</h3>
            <p>{t("models.ex.think.note")}</p>
          </div>
          <label className="switch-row" data-spot="reasoning">
            <span className="sw-text">
              <b>{t("models.ex.think.switch")}</b>
              <span>{t("models.ex.think.switch.note")}</span>
            </span>
            <input type="checkbox" role="switch" className="switch" checked={form.reasoning === "on"} onChange={(e) => put("reasoning", e.target.checked ? "on" : "off")} />
          </label>
        </section>
      </Row>

      <Row title={t("models.ex.start")} text={t("models.ex.start.note", { restart: restartText })} spot="start-options">
        {progress && <SwitchProgress progress={progress} engine={engine} />}
        {dropped.length > 0 && (
          <p className="pc-note mx-dropped">
            <Icon name="alert" small />
            <span>
              {rich("models.ex.dropped", {
                list: dropped.map((a, i) => (
                  <span key={`${a}-${i}`}>
                    {i > 0 && ", "}
                    <code className="macro">{a}</code>
                  </span>
                )),
              })}
            </span>
          </p>
        )}
        <div className="mx-knobs">
          <Slot
            id={`${base}-ctx`}
            label={t("models.sug.ctx")}
            note={t("models.ex.ctx.note")}
            spot="context"
            wide
            value={form.load.ctx ?? ""}
            placeholder={est?.ctx_train ? String(est.ctx_train) : t("models.profile.model")}
            changed={loadChanged.includes("ctx")}
            restart
            invalid={form.load.ctx !== null && !START_BY.ctx.ok(form.load.ctx)}
            onText={(v) => setLoad("ctx", v)}
            slider={
              steps.length > 1
                ? {
                    min: 0,
                    max: steps.length - 1,
                    step: 1,
                    value: Math.max(0, stepAt === -1 ? steps.length - 1 : stepAt),
                    text: ctxNow ? t("models.ex.tokens", { v: ctxShort(ctxNow) }) : undefined,
                    onChange: (i) => setLoad("ctx", String(steps[i].ctx)),
                    ticks: [ctxShort(steps[0].ctx), ctxShort(steps[steps.length - 1].ctx)],
                  }
                : undefined
            }
          >
            <Memory step={step} est={est} error={estError} />
          </Slot>
          <Slot
            id={`${base}-ngl`}
            label={t("models.ex.load.ngl")}
            note={layers ? t("models.ex.ngl.noteLayers", { n: layers }) : t("models.ex.ngl.note")}
            spot="gpu-layers"
            value={form.load.ngl ?? ""}
            placeholder="auto"
            changed={loadChanged.includes("ngl")}
            restart
            invalid={form.load.ngl !== null && !START_BY.ngl.ok(form.load.ngl)}
            onText={(v) => setLoad("ngl", v)}
            slider={numSlider(form.load.ngl, 0, layers ?? 99, 1, (n) => setLoad("ngl", String(n)), layers ?? 99)}
          />
          <Slot
            id={`${base}-ncmoe`}
            label={t("models.ex.ncmoe")}
            note={t("models.ex.ncmoe.note")}
            spot="n-cpu-moe"
            value={form.load.ncmoe ?? ""}
            placeholder="0"
            changed={loadChanged.includes("ncmoe")}
            restart
            invalid={form.load.ncmoe !== null && !START_BY.ncmoe.ok(form.load.ncmoe)}
            onText={(v) => setLoad("ncmoe", v)}
            slider={numSlider(form.load.ncmoe, 0, layers ?? 99, 1, (n) => setLoad("ncmoe", String(n)), 0)}
          />
          <div className="mx-knob mx-two" data-spot="kv-cache">
            {(["ctk", "ctv"] as const).map((key) => (
              <div key={key} className="mx-knob-top">
                <label className="mx-knob-label" htmlFor={`${base}-${key}`}>
                  <b>
                    {key === "ctk" ? "KV-Cache K" : "KV-Cache V"}
                    <RestartTag on={loadChanged.includes(key)} />
                  </b>
                  <small>{key === "ctk" ? t("models.ex.ctk.note") : t("models.ex.ctv.note")}</small>
                </label>
                <select id={`${base}-${key}`} className="input ms-select mx-select" value={form.load[key] ?? ""} onChange={(e) => setLoad(key, e.target.value)}>
                  <option value="">{t("models.ex.kvDefault")}</option>
                  {[...KV_TYPES, ...(form.load[key] && !KV_TYPES.includes(form.load[key]!) ? [form.load[key]!] : [])].map((v) => (
                    <option key={v} value={v}>
                      {v}
                    </option>
                  ))}
                </select>
              </div>
            ))}
          </div>
          <Choice
            label="Flash-Attention"
            note={t("models.ex.fa.note")}
            spot="flash-attn"
            value={form.load.fa}
            changed={loadChanged.includes("fa")}
            options={[
              [null, t("models.folders.default")],
              ["auto", t("models.ex.auto")],
              ["on", t("models.ex.on")],
              ["off", t("models.ex.off")],
            ]}
            onPick={(v) => setLoad("fa", v)}
          />
          <Slot
            id={`${base}-threads`}
            label={t("models.ex.load.threads")}
            note={t("models.ex.threads.note")}
            spot="threads"
            value={form.load.threads ?? ""}
            placeholder="auto"
            changed={loadChanged.includes("threads")}
            restart
            invalid={form.load.threads !== null && !START_BY.threads.ok(form.load.threads)}
            onText={(v) => setLoad("threads", v)}
            slider={numSlider(form.load.threads, 1, 64, 1, (n) => setLoad("threads", String(n)), 8)}
          />
          <Slot
            id={`${base}-batch`}
            label={t("models.ex.load.batch")}
            note={t("models.ex.batch.note")}
            spot="batch"
            value={form.load.batch ?? ""}
            placeholder="2048"
            changed={loadChanged.includes("batch")}
            restart
            invalid={form.load.batch !== null && !START_BY.batch.ok(form.load.batch)}
            onText={(v) => setLoad("batch", v)}
            slider={numSlider(form.load.batch, 64, 8192, 64, (n) => setLoad("batch", String(n)), 2048)}
          />
          <Slot
            id={`${base}-ub`}
            label={t("models.ex.load.ub")}
            note={t("models.ex.ub.note")}
            spot="micro-batch"
            value={form.load.ub ?? ""}
            placeholder="512"
            changed={loadChanged.includes("ub")}
            restart
            invalid={form.load.ub !== null && !START_BY.ub.ok(form.load.ub)}
            onText={(v) => setLoad("ub", v)}
            slider={numSlider(form.load.ub, 32, 4096, 32, (n) => setLoad("ub", String(n)), 512)}
          />
          <Choice
            label="mmap"
            note={t("models.ex.mmap.note")}
            spot="mmap"
            value={form.load.mmap}
            changed={loadChanged.includes("mmap")}
            options={[
              [null, t("models.folders.default")],
              ["on", t("models.ex.on")],
              ["off", t("models.ex.off")],
            ]}
            onPick={(v) => setLoad("mmap", v)}
          />
          <label className="switch-row mx-switch" data-spot="mlock">
            <span className="sw-text">
              <b>
                mlock
                <RestartTag on={loadChanged.includes("mlock")} />
              </b>
              <span>{t("models.ex.mlock.note")}</span>
            </span>
            <input type="checkbox" role="switch" className="switch" checked={form.load.mlock === "on"} onChange={(e) => setLoad("mlock", e.target.checked ? "on" : null)} />
          </label>
          <div className="mx-knob mx-wide" data-spot="extra-args">
            <label className="mx-knob-label" htmlFor={`${base}-rest`}>
              <b>
                {t("models.ex.rest")}
                <RestartTag on={restChanged} />
              </b>
              <small>{t("models.ex.rest.note")}</small>
            </label>
            <textarea
              id={`${base}-rest`}
              className="textarea mx-rest num"
              rows={Math.min(8, Math.max(3, form.rest.split("\n").length + 1))}
              spellCheck={false}
              autoComplete="off"
              value={form.rest}
              onChange={(e) => put("rest", e.target.value)}
            />
          </div>
        </div>
      </Row>

      <div className={`savebar mx-save${dirty ? " dirty" : ""}${!dirty && !error && !bad && state === "idle" ? " quiet" : ""}`} role="group" aria-label={t("models.save")}>
        <span className={error || bad ? "bad" : "muted"} role="status">
          {error ||
            bad ||
            (state === "saving"
              ? t("models.saving")
              : dirty
                ? launchChanged
                  ? t("models.ex.unsaved", {
                      list: [
                        ...new Set([
                          ...loadChanged.map((k) => LOAD_LABEL[k]),
                          ...(restChanged ? [t("models.ex.rest")] : []),
                          ...(form.file !== start.file ? [t("models.ex.file")] : []),
                          ...(form.engineId !== start.engineId ? [t("models.ex.engine")] : []),
                          ...(samplerChanged ? [t("models.ex.answer")] : []),
                        ]),
                      ].join(", "),
                      restart: restartText,
                    })
                  : t("models.ex.unsaved.sampler")
                : state === "saved"
                  ? t("models.ex.saved")
                  : t("models.ex.noChanges", { name: p.name }))}
        </span>
        <button type="button" className="btn ghost" onClick={discard} disabled={!dirty || state === "saving"}>
          {t("models.ex.discard")}
        </button>
        <button type="button" className="btn primary" onClick={() => void save()} disabled={!dirty || state === "saving" || Boolean(bad)}>
          {state === "saving" ? t("models.saving") : t("models.save")}
        </button>
      </div>
    </>
  );
}

/** The rules of the start options with a number. */
const isInt = (v: string, min: number) => /^-?\d+$/.test(v) && Number(v) >= min;
type StartRule = { key: LoadKey; readonly label: string; readonly rule: string; ok: (v: string) => boolean };
/** `label` and `rule` are read in the language on screen each time (`texts`). */
const START: StartRule[] = [
  texts({ key: "ctx" as LoadKey, ok: (v: string) => isInt(v, 0) }, { label: "models.sug.ctx", rule: "models.ex.rule.from0" }),
  texts({ key: "ngl" as LoadKey, ok: (v: string) => isInt(v, 0) || v === "auto" || v === "all" }, { label: "models.ex.load.ngl", rule: "models.ex.rule.ngl" }),
  texts({ key: "ncmoe" as LoadKey, ok: (v: string) => isInt(v, 0) }, { label: "models.ex.load.ncmoe", rule: "models.ex.rule.from0" }),
  texts({ key: "threads" as LoadKey, ok: (v: string) => isInt(v, 1) || v === "-1" }, { label: "models.ex.load.threads", rule: "models.ex.rule.from1" }),
  texts({ key: "batch" as LoadKey, ok: (v: string) => isInt(v, 1) }, { label: "models.ex.load.batch", rule: "models.ex.rule.from1" }),
  texts({ key: "ub" as LoadKey, ok: (v: string) => isInt(v, 1) }, { label: "models.ex.load.ub", rule: "models.ex.rule.from1" }),
];
const START_BY = Object.fromEntries(START.map((s) => [s.key, s])) as Record<"ctx" | "ngl" | "ncmoe" | "threads" | "batch" | "ub", StartRule>;

type Slider = { min: number; max: number; step: number; value: number; text?: string; onChange: (n: number) => void; ticks?: [string, string] };

/** A slider over `min…max` for a start option kept as text (an unset or word value shows `dflt`). */
function numSlider(v: string | null, min: number, max: number, step: number, onChange: (n: number) => void, dflt: number): Slider {
  const n = v !== null && /^-?\d+$/.test(v) ? Number(v) : dflt;
  return { min, max, step, value: Math.min(max, Math.max(min, n)), onChange, ticks: [num(min), num(max)] };
}

function RestartTag({ on }: { on: boolean }) {
  return on ? (
    <span className="tag ms-restart on" title={t("models.ex.restartTag.title")}>
      {t("models.ex.restartTag")}
    </span>
  ) : null;
}

type SlotProps = {
  id: string;
  label: string;
  note: string;
  spot?: string;
  value: string;
  placeholder: string;
  changed: boolean;
  restart?: boolean;
  invalid: boolean;
  wide?: boolean;
  decimal?: boolean;
  onText: (v: string) => void;
  slider?: Slider;
  children?: ReactNode;
};

/** One value: its name and line, a field, "Standard" (leaves it out) and a slider under them. */
function Slot({ id, label, note, spot, value, placeholder, changed, restart, invalid, wide, decimal, onText, slider, children }: SlotProps) {
  return (
    <div className={`mx-knob${changed ? " changed" : ""}${wide ? " mx-wide" : ""}`} data-spot={spot}>
      <div className="mx-knob-top">
        <label className="mx-knob-label" htmlFor={id}>
          <b>
            {label}
            {restart && <RestartTag on={changed} />}
          </b>
          <small>{note}</small>
        </label>
        <div className="mx-knob-ctl">
          <input
            id={id}
            className={`input mx-num num${invalid ? " invalid" : ""}`}
            // Sampler values are numbers; start options are text, so a word like "auto" stays possible.
            type={decimal === undefined ? "text" : "number"}
            inputMode={decimal ? "decimal" : "numeric"}
            value={value}
            placeholder={placeholder}
            aria-invalid={invalid || undefined}
            autoComplete="off"
            onChange={(e) => onText(e.target.value.trim())}
          />
          {value === "" ? (
            <span className="mx-std" title={t("models.ex.std.title")}>
              {t("models.folders.default")}
            </span>
          ) : (
            <button type="button" className="btn sm ghost mx-reset" onClick={() => onText("")} aria-label={t("models.ex.std.aria", { label })} title={t("models.ex.std.reset")}>
              <Icon name="rewind" small />
              {t("models.folders.default")}
            </button>
          )}
        </div>
      </div>
      {slider && (
        <>
          <input
            type="range"
            className="mx-range"
            aria-label={label}
            min={slider.min}
            max={slider.max}
            step={slider.step}
            value={slider.value}
            aria-valuetext={slider.text}
            onChange={(e) => slider.onChange(Number(e.target.value))}
          />
          {slider.ticks && (
            <div className="mx-ticks num" aria-hidden="true">
              <span>{slider.ticks[0]}</span>
              <span>{slider.ticks[1]}</span>
            </div>
          )}
        </>
      )}
      {children}
    </div>
  );
}

function SamplerKnob({ k, id, text, changed, onText }: { k: Knob; id: string; text: string; changed: boolean; onText: (v: string) => void }) {
  const v = text === "" ? k.dflt : Number(text);
  const step = k.slide ?? k.step;
  const places = decimals(k.step);
  return (
    <Slot
      id={id}
      label={k.label}
      note={k.note}
      spot={k.spot}
      value={text}
      placeholder={num(k.dflt)}
      changed={changed}
      invalid={badValue(k, text) !== null}
      decimal={k.step < 1}
      onText={onText}
      slider={{
        min: k.range[0],
        max: k.range[1],
        step,
        value: Number.isFinite(v) ? Math.min(k.range[1], Math.max(k.range[0], v)) : k.dflt,
        text: Number.isFinite(v) ? num(v) : undefined,
        onChange: (n) => onText(String(Number(n.toFixed(places)))),
        ticks: [num(k.range[0]), num(k.range[1])],
      }}
    />
  );
}

/** A start option with a few values, as a segmented choice; null is "Standard" (left out). */
function Choice({ label, note, spot, value, changed, options, onPick }: { label: string; note: string; spot: string; value: string | null; changed: boolean; options: [string | null, string][]; onPick: (v: string | null) => void }) {
  const id = useId();
  const known = options.some(([v]) => v === value);
  return (
    <div className={`mx-knob${changed ? " changed" : ""}`} data-spot={spot}>
      <div className="mx-knob-top">
        <span className="mx-knob-label" id={id}>
          <b>
            {label}
            <RestartTag on={changed} />
          </b>
          <small>
            {note}
            {!known && value !== null && ` ${t("models.ex.now", { value: value || t("models.ex.noValue") })}`}
          </small>
        </span>
      </div>
      <div className="seg mx-seg" role="group" aria-labelledby={id}>
        {options.map(([v, text]) => (
          <button key={text} type="button" aria-pressed={value === v} data-spot-focus={value === v || undefined} onClick={() => onPick(v)}>
            {text}
          </button>
        ))}
      </div>
    </div>
  );
}

/** "ca. 9,4 GB VRAM · frei 18,2 von 24,0 GB" and whether it fits, for the chosen context. */
function Memory({ step, est, error }: { step: Estimate["steps"][number] | null; est: Estimate | null; error: string }) {
  if (error) return <p className="help">{t("models.ex.mem.unknown", { error })}</p>;
  if (!est) return <p className="help ms-mem">{t("models.ex.mem.busy")}</p>;
  if (!step) return <p className="help ms-mem">{t("models.ex.mem.none")}</p>;
  const fit = step.fit;
  return (
    <div className="ms-mem">
      <p className="num">
        <b>{t("models.ex.mem.vram", { size: gb(step.gpu_bytes) })}</b>
        {step.cpu_bytes > 0 && <> · {t("models.place.ram", { ram: gb(step.cpu_bytes) })}</>}
        {est.vram_free_bytes !== null && est.vram_bytes !== null && (
          <span className="faint">
            {" "}
            · {t("models.ex.mem.free", { free: gb(est.vram_free_bytes), all: gb(est.vram_bytes) })}
          </span>
        )}
      </p>
      {fit !== "unknown" && (
        <p className="ms-fit">
          <span className={`tag fit-${fit}`}>{FIT_TEXT[fit]}</span>
          <span className="faint">{FIT_NOTE[fit]}</span>
        </p>
      )}
    </div>
  );
}

