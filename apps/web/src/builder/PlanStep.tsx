// Step 1 of the card builder: the idea in one sentence and a few anchors; from them an outline that
// shows which part of the plan lands in which field of the card.

import type { CharInfo } from "../api";
import { t } from "../i18n";
import { Icon } from "../ui";
import { outline, type Plan } from "./draft";

type Props = {
  plan: Plan;
  onPlan: (patch: Partial<Plan>) => void;
  /** The library's characters, for "start from an existing one" (null while it loads). */
  characters: CharInfo[] | null;
  onTemplate: (id: number) => void;
  /** Lays the outline into the empty fields and goes on to building. */
  onUseOutline: () => void;
};

/** A row of suggestions under a field; a click sets the field (a second click on the same one clears it). */
function Picks({ words, value, onPick, label }: { words: string; value: string; onPick: (v: string) => void; label: string }) {
  const list = words.split(",").map((w) => w.trim()).filter(Boolean);
  return (
    <div className="cb-picks" role="group" aria-label={label}>
      {list.map((w) => {
        const on = value.trim().toLowerCase() === w.toLowerCase();
        return (
          <button type="button" key={w} className={`cb-pick${on ? " on" : ""}`} aria-pressed={on} onClick={() => onPick(on ? "" : w)}>
            {w}
          </button>
        );
      })}
    </div>
  );
}

export function PlanStep({ plan, onPlan, characters, onTemplate, onUseOutline }: Props) {
  const rows = outline(plan);
  return (
    <>
      <section className="set-row">
        <div className="set-label">
          <h2>{t("builder.plan.title")}</h2>
          <p>{t("builder.plan.text")}</p>
        </div>
        <div className="set-panel">
          <label className="field">
            <span className="field-label">{t("builder.plan.idea")}</span>
            <span className="help">{t("builder.plan.idea.help")}</span>
            <textarea className="textarea cb-idea" rows={2} value={plan.idea} placeholder={t("builder.plan.idea.example")} onChange={(e) => onPlan({ idea: e.target.value })} />
          </label>
          <div className="cb-two">
            <div className="field">
              <label className="field-label" htmlFor="cb-genre">
                {t("builder.plan.genre")}
              </label>
              <input id="cb-genre" className="input" value={plan.genre} onChange={(e) => onPlan({ genre: e.target.value })} />
              <Picks words={t("builder.plan.genres")} value={plan.genre} onPick={(genre) => onPlan({ genre })} label={t("builder.plan.genre")} />
            </div>
            <div className="field">
              <label className="field-label" htmlFor="cb-tone">
                {t("builder.plan.tone")}
              </label>
              <input id="cb-tone" className="input" value={plan.tone} onChange={(e) => onPlan({ tone: e.target.value })} />
              <Picks words={t("builder.plan.tones")} value={plan.tone} onPick={(tone) => onPlan({ tone })} label={t("builder.plan.tone")} />
            </div>
          </div>
          <label className="field">
            <span className="field-label">{t("builder.plan.relation")}</span>
            <span className="help">{t("builder.plan.relation.help", { user: "{{user}}" })}</span>
            <input className="input" value={plan.relation} placeholder={t("builder.plan.relation.example")} onChange={(e) => onPlan({ relation: e.target.value })} />
          </label>
          <label className="field">
            <span className="field-label">{t("builder.plan.traits")}</span>
            <span className="help">{t("builder.plan.traits.help")}</span>
            <input
              className="input"
              value={plan.traits.join(", ")}
              placeholder={t("builder.plan.traits.example")}
              onChange={(e) => onPlan({ traits: e.target.value.split(",").map((x) => x.trimStart()) })}
              onBlur={() => onPlan({ traits: plan.traits.map((x) => x.trim()).filter(Boolean) })}
            />
          </label>
          <label className="field">
            <span className="field-label">{t("builder.plan.drive")}</span>
            <span className="help">{t("builder.plan.drive.help")}</span>
            <textarea className="textarea" rows={2} value={plan.drive} onChange={(e) => onPlan({ drive: e.target.value })} />
          </label>
          <div className="cb-two">
            <label className="field">
              <span className="field-label">{t("builder.plan.voice")}</span>
              <input className="input" value={plan.voice} placeholder={t("builder.plan.voice.example")} onChange={(e) => onPlan({ voice: e.target.value })} />
            </label>
            <label className="field">
              <span className="field-label">{t("builder.plan.world")}</span>
              <input className="input" value={plan.world} placeholder={t("builder.plan.world.example")} onChange={(e) => onPlan({ world: e.target.value })} />
            </label>
          </div>
        </div>
      </section>

      <section className="set-row">
        <div className="set-label">
          <h2>{t("builder.outline.title")}</h2>
          <p>{t("builder.outline.text")}</p>
        </div>
        <div className="set-panel">
          <ol className="cb-outline">
            {rows.map((r) => (
              <li key={r.field}>
                <b>{r.label}</b>
                <span>{r.parts.join(" · ")}</span>
              </li>
            ))}
          </ol>
          <div className="btn-row">
            <button type="button" className="btn primary" onClick={onUseOutline}>
              <Icon name="next" small />
              {t("builder.outline.use")}
            </button>
          </div>
          <p className="help">{t("builder.outline.use.help")}</p>
        </div>
      </section>

      {characters && characters.length > 0 && (
        <section className="set-row">
          <div className="set-label">
            <h2>{t("builder.template.title")}</h2>
            <p>{t("builder.template.text")}</p>
          </div>
          <div className="set-panel">
            <label className="field">
              <span className="field-label">{t("builder.template.pick")}</span>
              <select
                className="input"
                value=""
                onChange={(e) => {
                  const id = Number(e.target.value);
                  if (id) onTemplate(id);
                }}
              >
                <option value="">{t("builder.template.none")}</option>
                {characters.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </section>
      )}
    </>
  );
}
