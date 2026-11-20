// Small parts the tab "Vorschläge" uses in several places: the five-dot rating, the fit pill, the
// action of one size tier ("Herunterladen", the progress, or "Installiert" with "Benutzen") and a
// phone check that follows the window.

import { useEffect, useState } from "react";
import { api, ApiError, errorText, postJson, type ActivateAnswer, type LocalModels, type Profile } from "../api";
import { tellModelNews } from "../app/engineEvents";
import { t } from "../i18n";
import { Icon } from "../ui";
import { downloadOf, errorOf, gb, size, startDownload, TIER_NAME, useDownloads } from "./store";
import type { CatalogModel, CatalogQuant, Placement, Tier } from "./types";
import { fallbackText, fitName, percent, placementText, splitTitle } from "./words";

const PHONE = "(max-width: 767px)";

/** True while the window is phone-wide. */
export function usePhone(): boolean {
  const [phone, setPhone] = useState(() => matchMedia(PHONE).matches);
  useEffect(() => {
    const mq = matchMedia(PHONE);
    const on = () => setPhone(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return phone;
}

/** "Rollenspiel ●●●●○": a rating 1–5, read out as "Rollenspiel 4 von 5". */
export function Dots({ label, value }: { label: string; value: number }) {
  return (
    <span className="sug-rate" role="img" aria-label={t("models.dots", { label, v: value })}>
      <span className="sug-rate-l">{label}</span>
      <span className="sug-dots" aria-hidden="true">
        {[1, 2, 3, 4, 5].map((i) => (
          <i key={i} className={i <= value ? "on" : undefined} />
        ))}
      </span>
    </span>
  );
}

export function FitPill({ q }: { q: CatalogQuant }) {
  if (!q.fit) return <span className="fit-pill">{t("models.about", { size: gb(q.est_gb) })}</span>;
  // With RAM help the parts as this PC would hold them; otherwise the need all on the GPU.
  const split =
    q.fit === "cpu_help" && q.ram_gb != null
      ? q.gpu_gb
        ? t("models.pill.split", { gpu: gb(q.gpu_gb), ram: gb(q.ram_gb) })
        : t("models.pill.ram", { ram: gb(q.ram_gb) })
      : gb(q.est_gb);
  return (
    <span className={`fit-pill fit-${q.fit}`} title={splitTitle(q)}>
      {fitName(q)} <span className="num">· {t("models.about", { size: split })}</span>
    </span>
  );
}

/** What the server answered to "use", for the cases that do not restart anything. */
type Quiet = Exclude<ActivateAnswer, "restarting">;
const USE_NOTE: { [A in Quiet]: `models.use.${A}` } = {
  unchanged: "models.use.unchanged",
  unmanaged: "models.use.unmanaged",
  other_window: "models.use.other_window",
  foreign: "models.use.foreign",
  port_busy: "models.use.port_busy",
};

const fileName = (p: string) => (p.split(/[\\/]/).pop() ?? p).toLowerCase();

/** Runs the installed file `file`: through "Meine Modelle" (`/models/local/use`), or, for a file only a
 *  model profile knows (outside the model folders), by activating that profile. */
async function useFile(file: string): Promise<{ id: number; engine: ActivateAnswer; placement?: Placement | null } | null> {
  const name = fileName(file);
  const local = await api<LocalModels>("/models/local");
  const hit = local.models.find((m) => fileName(m.path) === name || m.file.toLowerCase() === name);
  if (hit) {
    // `placement`: how a new profile was set up on this PC (null for a profile that already existed).
    const r = await postJson<{ profile: Profile; engine: ActivateAnswer; placement: Placement | null }>("/models/local/use", { path: hit.path });
    return { id: r.profile.id, engine: r.engine, placement: r.placement };
  }
  const { profiles } = await api<{ profiles: Profile[] }>("/models/profiles");
  const p = profiles.find((x) => fileName(x.model_file) === name);
  if (!p) return null;
  const r = await api<{ profile: Profile; engine: ActivateAnswer }>(`/models/profiles/${p.id}/activate`, { method: "POST" });
  return { id: r.profile.id, engine: r.engine };
}

/** "Benutzen" for an installed file, and what came of it. */
function UseButton({ m, tier, q, small }: { m: CatalogModel; tier: Tier; q: CatalogQuant; small?: boolean }) {
  const [busy, setBusy] = useState(false);
  // `loading`: the model switches now, so the note links to the tab "Modell".
  const [note, setNote] = useState<{ text: string; bad: boolean; loading?: boolean } | null>(null);
  const [placed, setPlaced] = useState<Placement | null>(null);
  const go = async () => {
    setBusy(true);
    setNote(null);
    setPlaced(null);
    try {
      const r = await useFile(q.file);
      if (!r) setNote({ text: t("models.use.gone"), bad: true });
      else {
        setPlaced(r.placement ?? null);
        if (r.engine === "restarting") {
          tellModelNews({ type: "engine", event: "switching", profile_id: r.id });
          setNote({ text: t("models.use.loading"), bad: false, loading: true });
        } else setNote({ text: t(USE_NOTE[r.engine]), bad: false });
      }
    } catch (e) {
      const busyNow = e instanceof ApiError && e.code === "busy";
      setNote({ text: busyNow ? t("models.use.busy") : t("models.use.failed", { error: errorText(e) }), bad: true });
    } finally {
      setBusy(false);
    }
  };
  return (
    <span className="sug-use">
      <span className="sug-use-row">
        <span className="sg-installed">
          <Icon name="check" small />
          {t("models.installed")}
        </span>
        <button type="button" className={`btn${small ? " sm" : ""}`} disabled={busy} onClick={() => void go()} aria-label={t("models.use.aria", { name: m.name, tier: TIER_NAME[tier] })}>
          {t("models.use")}
        </button>
      </span>
      {note && (
        <span className={note.bad ? "bad" : "sug-note"} role="status">
          {note.text}
          {note.loading && (
            <>
              {" "}
              <a className="link" href="#/settings/model">
                {t("models.use.toModelTab")}
              </a>
            </>
          )}
        </span>
      )}
      {placed && (
        <span className="sug-note r1-placed" title={splitTitle(placed)}>
          {placementText(placed)}
          {fallbackText(placed.fallback) && <span className="r1-fallback">{fallbackText(placed.fallback)}</span>}
        </span>
      )}
    </span>
  );
}

/** The action of one tier: "Herunterladen", the progress while it loads, or "Installiert" + "Benutzen". */
export function GetButton({ m, tier, q, small }: { m: CatalogModel; tier: Tier; q: CatalogQuant; small?: boolean }) {
  const { list } = useDownloads();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const d = downloadOf(list, m.repo, q.file);
  const cls = `btn${small ? " sm" : ""}`;
  if (q.installed || d?.state === "done") return <UseButton m={m} tier={tier} q={q} small={small} />;
  if (d) {
    const pct = d.total ? Math.floor((d.done / d.total) * 100) : 0;
    return (
      <a className={`${cls} ghost`} href="#/settings/suggestions" onClick={() => document.querySelector('[data-spot="downloads"]')?.scrollIntoView({ block: "center" })}>
        {d.state === "running"
          ? t("models.get.running", { pct: percent(pct) })
          : d.state === "queued"
            ? t("models.dl.state.queued")
            : d.state === "paused"
              ? t("models.get.paused", { pct: percent(pct) })
              : t("models.dl.state.failed")}
      </a>
    );
  }
  if (m.gated) return <span className="help">{t("models.get.gated")}</span>;
  if (q.missing) return <span className="help">{t("models.get.missing")}</span>;
  const start = async () => {
    setBusy(true);
    setError("");
    try {
      await startDownload({ repo: m.repo, file: q.file, size: q.size_bytes, sha256: q.sha256 });
    } catch (e) {
      setError(errorOf(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <span className="sg-get">
      <button type="button" className={cls} disabled={busy} onClick={() => void start()} aria-label={t("models.get.aria", { name: m.name, tier: TIER_NAME[tier], size: size(q.size_bytes) })}>
        <Icon name="download" small />
        {t("models.get")}
      </button>
      {error && <span className="bad">{error}</span>}
    </span>
  );
}
