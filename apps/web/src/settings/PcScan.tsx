// Settings tab "Modell", lower part: "Dein PC" (the hardware scan with the suggested llama.cpp build)
// and "Engines" (the llama-server builds: the user's own and official ones the app downloaded,
// with their self-test, and the download of a new one from the llama.cpp releases on GitHub).
// The scan is only a suggestion: nothing is switched by it.

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { ApiError, api, engineName, errorText, postJson, type Engine } from "../api";
import { dateFormat, numberFormat, t, texts } from "../i18n";
import { rich } from "../i18n/rich";
import { withUnit } from "../models/words";
import { useEngineDownload, type EngineDownloadEvent, type SelfTest } from "../app/engineDownload";
import { useModelNews } from "../app/engineEvents";
import { PcOnly } from "../phone/Phone";
import { respot } from "../spot";
import { Icon } from "../ui";

type Gpu = {
  name: string;
  vendor: "amd" | "nvidia" | "intel" | "other";
  vram_total: number;
  /** Dedicated VRAM other programs hold now (the app's own model server not counted); null: unknown. */
  vram_used_by_others: number | null;
  driver: string | null;
  /** A known problem driver by its code (crates/hw/src/driver.rs); null: none known. */
  driver_issue: string | null;
};

/** A known problem driver in words; null for a code this version of the app has no words for. */
function driverIssue(gpu: Gpu): string | null {
  const v = gpu.driver ?? "";
  return gpu.driver_issue === "amd_adrenalin_26_5" ? t("models.pc.driverIssue.amd_adrenalin_26_5", { v }) : null;
}

export type Scan = {
  gpus: Gpu[];
  ram_total: number;
  ram_free: number;
  commit: { limit: number; used: number; warning: string | null };
  cpu: { name: string; cores: number; threads: number };
  backend: {
    suggested: "rocm" | "cuda" | "vulkan" | "cpu";
    cuda: string | null;
    gpu_index: number | null;
    reason_code: string;
    reason: string;
    rocm_runtime_found: boolean;
    rocm_runtime_path: string | null;
  };
  scanned_at: number;
};

export type Asset = { name: string; variant: string; backend: string; size: number; sha256: string; runtime: { name: string; size: number } | null };
export type Release = { tag: string; published_at: string; assets: Asset[] };
type EngineRow = Engine & { selftest?: SelfTest | null };

const BACKEND: Record<string, string> = { cpu: "CPU", rocm: "ROCm", cuda: "CUDA", vulkan: "Vulkan", metal: "Metal", sycl: "SYCL" };

/** What the server's refusals mean here (the shared ones come from `errorText`); read in the language
 *  on screen each time, `undefined` for a code without words here. */
export const ERRORS: Record<string, string> = texts(
  {},
  {
    scan_unsupported: "models.pc.err.scan_unsupported",
    scan_failed: "models.pc.err.scan_failed",
    github_unreachable: "models.pc.err.github_unreachable",
    download_running: "models.pc.err.download_running",
    engine_exists: "models.pc.err.engine_exists",
    rocm_runtime_missing: "models.pc.err.rocm_runtime_missing",
    release_missing: "models.pc.err.release_missing",
    engine_own: "models.pc.err.engine_own",
    engine_in_use: "models.pc.err.engine_in_use",
    engine_missing: "models.pc.err.engine_missing",
    // Why a download failed (from the socket event).
    hash_mismatch: "models.pc.err.hash_mismatch",
    no_space: "models.pc.err.no_space",
    bad_zip: "models.pc.err.bad_zip",
    download_failed: "models.pc.err.download_failed",
    selftest_failed: "models.pc.err.selftest_failed",
    rocm_no_gpu: "models.pc.err.rocm_no_gpu",
    bad_asset: "models.pc.err.bad_asset",
  },
);

export const say = (e: unknown) => (e instanceof ApiError && ERRORS[e.code]) || errorText(e);

const GIB = 1024 ** 3;
const gb = (bytes: number) => withUnit(Math.round((bytes / GIB) * 10) / 10, "gigabyte", 1, 0);
const mb = (bytes: number) => withUnit(Math.round(bytes / 1e6), "megabyte", 0);
const when = (secs: number) => dateFormat({ day: "numeric", month: "numeric", year: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(secs * 1000));

/** "Treiber 32.0.31036.15" without the vendor's own notation. */
const vendorName = (v: Gpu["vendor"]) => (v === "amd" ? "AMD" : v === "nvidia" ? "NVIDIA" : v === "intel" ? "Intel" : t("models.pc.otherVendor"));

/** The suggestion's reason in the app's language, from the server's rule code (`backend_suggestion`
 *  in `crates/hw/src/backend.rs`); the driver release is in the server's own sentence. A code this
 *  version does not know keeps the server's sentence. */
function reasonOf(b: Scan["backend"]): string {
  const major = /\d{3}/.exec(b.reason)?.[0] ?? "";
  switch (b.reason_code) {
    case "no_gpu":
      return t("models.pc.reason.no_gpu");
    case "nvidia_cuda13":
      return t("models.pc.reason.nvidia_cuda13", { major });
    case "nvidia_cuda12":
      // The same rule names a card before Turing on a new driver ("vor RTX 20").
      return b.reason.includes("RTX 20") ? t("models.pc.reason.nvidia_cuda12.old_card", { major }) : t("models.pc.reason.nvidia_cuda12", { major });
    case "nvidia_driver_old":
      return t("models.pc.reason.nvidia_driver_old", { major });
    case "nvidia_driver_unknown":
      return t("models.pc.reason.nvidia_driver_unknown");
    case "amd_rocm":
      return t("models.pc.reason.amd_rocm");
    case "amd_no_rocm":
      return t("models.pc.reason.amd_no_rocm");
    case "intel":
      return t("models.pc.reason.intel");
    case "other_vendor":
      return t("models.pc.reason.other_vendor");
    default:
      return b.reason;
  }
}

/** The server's notice that virtual memory is nearly full (`commit_warning` in `crates/hw`), from its
 *  numbers, in the app's language. */
function commitWarning(c: Scan["commit"]): string | null {
  if (!c.warning) return null;
  const pct = Math.floor((c.used * 100) / Math.max(1, c.limit));
  const free = numberFormat({ maximumFractionDigits: 1, minimumFractionDigits: 1 }).format(Math.max(0, c.limit - c.used) / 1e9);
  return t("models.pc.commitWarning", { pct: String(pct), free });
}

export function PcScan() {
  const [scan, setScan] = useState<Scan | null | undefined>(undefined);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    api<{ scan: Scan | null }>("/system/scan")
      .then((r) => setScan(r.scan))
      .catch((e: unknown) => {
        setScan(null);
        setError(say(e));
      });
  }, []);

  const arrived = scan !== undefined;
  useEffect(() => {
    if (arrived) respot();
  }, [arrived]);

  const run = async () => {
    setScanning(true);
    setError("");
    try {
      setScan(await api<Scan>("/system/scan", { method: "POST" }));
    } catch (e) {
      setError(say(e));
    } finally {
      setScanning(false);
    }
  };

  return (
    <>
      <div className="set-row">
        <div className="set-label">
          <h2>{t("models.pc.title")}</h2>
          <p>{t("models.pc.note")}</p>
        </div>
        {/* The place for Ctrl+K exists once its content does, so the focus lands on the button. */}
        <div className="set-panel pc" data-spot={scan === undefined ? undefined : "pc"}>
          {scan === undefined ? (
            <div aria-busy="true">
              <span className="bar-ph" />
            </div>
          ) : scan === null ? (
            <div className="pc-empty">
              <p>{t("models.pc.notYet")}</p>
              <button type="button" className="btn primary" onClick={() => void run()} disabled={scanning} data-spot-focus>
                {scanning ? t("models.hf.busy") : t("models.pc.scan")}
              </button>
            </div>
          ) : (
            <ScanResult scan={scan} scanning={scanning} onScan={() => void run()} />
          )}
          {error && (
            <p className="bad" role="alert">
              {error}
            </p>
          )}
        </div>
      </div>
      <Engines scan={scan ?? null} />
    </>
  );
}

function ScanResult({ scan, scanning, onScan }: { scan: Scan; scanning: boolean; onScan: () => void }) {
  const b = scan.backend;
  const main = b.gpu_index !== null ? scan.gpus[b.gpu_index] : undefined;
  // The suggested card first, the others (integrated graphics) after it.
  const gpus = main ? [main, ...scan.gpus.filter((g) => g !== main)] : scan.gpus;
  const backend = b.suggested === "cuda" && b.cuda ? `CUDA ${b.cuda}` : BACKEND[b.suggested];
  return (
    <>
      <div className="mm-head">
        <div className="mm-title">
          <b className="mm-name">{main?.name ?? gpus[0]?.name ?? t("models.pc.noGpu")}</b>
          <span className="mm-file num">{t("models.pc.scannedAt", { when: when(scan.scanned_at) })}</span>
        </div>
        <button type="button" className="btn sm" onClick={onScan} disabled={scanning} data-spot-focus aria-label={t("models.pc.scan")}>
          <Icon name="rewind" small />
          {scanning ? t("models.hf.busy") : t("models.pc.rescan")}
        </button>
      </div>

      <div className="pc-pick" role="group" aria-label={t("models.pc.pick")}>
        <span className="pc-pick-head">{rich("models.pc.suggested", { backend: <b>{backend}</b> })}</span>
        <span className="pc-pick-why">{reasonOf(b)}</span>
      </div>

      {gpus.map((g, i) => (
        <GpuBlock key={`${g.name}-${i}`} gpu={g} named={g !== (main ?? gpus[0])} />
      ))}

      <dl className="mm-facts num">
        <Fact label={t("models.pc.ram")}>
          {gb(scan.ram_total)} <small>· {t("models.pc.free", { size: gb(scan.ram_free) })}</small>
        </Fact>
        <Fact label={t("models.pc.cpu")} title={scan.cpu.name}>
          {t("models.pc.cores", { n: scan.cpu.cores })} <small>· {t("models.pc.threads", { n: scan.cpu.threads })}</small>
        </Fact>
        <Fact label={t("models.pc.cpuModel")}>{scan.cpu.name || null}</Fact>
        <Fact label={t("models.pc.rocm")} title={b.rocm_runtime_path ?? undefined}>
          {b.rocm_runtime_found ? t("models.pc.found") : t("models.folders.notFound")}
        </Fact>
      </dl>
      {scan.commit.warning && <p className="pc-note">{commitWarning(scan.commit)}</p>}
    </>
  );
}

function GpuBlock({ gpu, named }: { gpu: Gpu; named: boolean }) {
  const used = gpu.vram_used_by_others;
  const share = used === null || gpu.vram_total === 0 ? 0 : Math.min(1, used / gpu.vram_total);
  const issue = driverIssue(gpu);
  return (
    <div className="pc-gpu" role="group" aria-label={t("models.pc.gpu", { name: gpu.name })}>
      {named && <b className="pc-gpu-name">{gpu.name}</b>}
      <div className="pc-vram num">
        <span>{rich("models.pc.vram", { size: <b>{gb(gpu.vram_total)}</b> })}</span>
        <span className="pc-vram-used">{used === null ? t("models.pc.usedUnknown") : t("models.pc.used", { size: gb(used) })}</span>
      </div>
      <div className="pc-bar" aria-hidden="true">
        <i style={{ transform: `scaleX(${share})` }} />
      </div>
      <p className="pc-driver num">
        {vendorName(gpu.vendor)} · {t("models.pc.driver", { v: gpu.driver ?? t("models.pc.unknown") })}
      </p>
      {issue && (
        <p className="pc-note" data-spot="driver">
          <Icon name="alert" small />
          <span>{issue}</span>
        </p>
      )}
    </div>
  );
}

function Fact({ label, children, title }: { label: string; children: ReactNode; title?: string }) {
  return (
    <div title={title}>
      <dt>{label}</dt>
      <dd>{children ?? <span className="faint">–</span>}</dd>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Engines

/** The folder a program lies in (`…\llama-b12387-rocm\llama-server.exe` → `llama-b12387-rocm`). */
const folderOf = (exe: string) => exe.split(/[\\/]/).slice(-2, -1)[0] ?? exe;

/** One line on what the self-test of an engine found. */
function testLine(e: EngineRow): string {
  if (!e.exists) return t("models.engine.missing");
  const test = e.selftest;
  if (!test) return e.origin === "own" ? `${folderOf(e.exe)} · ${t("models.engine.noTest")}` : t("models.engine.untested");
  if (test.ran.length === 0) return t("models.engine.vulkan");
  if (test.devices.length > 0) return `${t("models.engine.passed")} · ${test.devices.join(", ")}`;
  return t("models.engine.passed");
}

function Engines({ scan }: { scan: Scan | null }) {
  const [engines, setEngines] = useState<EngineRow[] | null>(null);
  const [error, setError] = useState("");
  const [open, setOpen] = useState(false);
  const [job, setJob] = useState<EngineDownloadEvent | null>(null);

  const load = useCallback(async () => {
    try {
      setEngines((await api<{ engines: EngineRow[] }>("/models/engines")).engines);
      setError("");
    } catch (e) {
      setError(say(e));
    }
  }, []);

  useEffect(() => {
    void load();
    // A download that runs (started on another device or before a reload) shows here too.
    api<{ job: EngineDownloadEvent | null }>("/system/engine-download")
      .then((r) => {
        if (r.job && r.job.phase !== "done" && r.job.phase !== "failed") {
          setJob(r.job);
          setOpen(true);
        }
      })
      .catch(() => {});
  }, [load]);

  const arrived = engines !== null;
  useEffect(() => {
    if (arrived) respot();
  }, [arrived]);

  const onNews = useCallback(
    (n: { type: string }) => {
      if (n.type === "models") void load();
    },
    [load],
  );
  useModelNews(onNews);
  const onDownload = useCallback((ev: EngineDownloadEvent) => {
    setJob(ev);
    setOpen(true);
  }, []);
  useEngineDownload(onDownload);

  return (
    <div className="set-row">
      <div className="set-label">
        <h2>{t("models.engines")}</h2>
        <p>{t("models.engines.note")}</p>
      </div>
      <div className="set-panel" data-spot={engines === null ? undefined : "engines"}>
        <PcOnly what={t("models.engines")}>
          {engines === null ? (
            !error && (
              <div aria-busy="true">
                <span className="bar-ph" />
              </div>
            )
          ) : (
            <>
              {engines.length === 0 && <p className="help">{t("models.engines.none")}</p>}
              <ul className="mp-list" aria-label={t("models.engines")}>
                {engines.map((e) => (
                  <EngineItem key={e.id} e={e} onChanged={load} />
                ))}
              </ul>
              {open ? (
                <Download scan={scan} job={job} installed={engines} onClose={() => setOpen(false)} onJob={setJob} />
              ) : (
                <div className="btn-row">
                  <button type="button" className="btn sm" onClick={() => setOpen(true)} data-spot-focus>
                    <Icon name="download" small />
                    {t("models.engine.get")}
                  </button>
                </div>
              )}
            </>
          )}
          {error && <p className="bad">{error}</p>}
        </PcOnly>
      </div>
    </div>
  );
}

function EngineItem({ e, onChanged }: { e: EngineRow; onChanged: () => Promise<void> }) {
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState("");
  const official = e.origin === "official";
  const origin = official ? t("models.engine.official") : t("models.engine.own");
  const remove = async () => {
    try {
      await api(`/models/engines/${e.id}`, { method: "DELETE" });
      await onChanged();
    } catch (err) {
      setError(say(err));
      setAsking(false);
    }
  };
  const bad = !e.exists || e.selftest?.ok === false;
  const name = engineName(e);
  return (
    <li className="mp-row" aria-label={t("models.engine.row", { name })}>
      <div className="mp-top">
        <div className="mp-text">
          <span className="mp-title">
            <b>{name}</b>
            {name.toLowerCase() !== origin.toLowerCase() && <span className="tag">{origin}</span>}
            {e.backend !== "unknown" && <span className="tag">{BACKEND[e.backend] ?? e.backend}</span>}
            {e.version && !e.name.includes(e.version) && <span className="tag num">{e.version}</span>}
          </span>
          <span className={`mp-meta${bad ? " bad" : ""}`} title={e.exe}>
            {testLine(e)}
          </span>
        </div>
        {official && !asking && (
          <div className="mp-actions">
            <button type="button" className="btn sm ghost" onClick={() => setAsking(true)} aria-label={t("models.engine.remove.aria", { name })}>
              <Icon name="trash" small />
              {t("models.folders.remove")}
            </button>
          </div>
        )}
      </div>
      {asking && (
        <div className="mp-confirm" role="group" aria-label={t("models.engine.confirm.aria")}>
          <p>{t("models.engine.confirm", { name })}</p>
          <div className="btn-row">
            <button type="button" className="btn sm danger" onClick={() => void remove()} autoFocus>
              {t("models.engine.confirm.yes")}
            </button>
            <button type="button" className="btn sm ghost" onClick={() => setAsking(false)}>
              {t("models.dl.cancel")}
            </button>
          </div>
        </div>
      )}
      {error && <p className="bad mp-error">{error}</p>}
    </li>
  );
}

type Phase = EngineDownloadEvent["phase"];
const STEPS: { phase: Phase; readonly label: string }[] = [
  texts({ phase: "download" as Phase }, { label: "models.get" }),
  texts({ phase: "unpack" as Phase }, { label: "models.engine.step.unpack" }),
  texts({ phase: "test" as Phase }, { label: "models.engine.step.test" }),
  texts({ phase: "done" as Phase }, { label: "models.engine.step.done" }),
];

/** Which variant fits the scan (the CUDA one of the suggested major version). */
export function fits(a: Asset, scan: Scan | null): boolean {
  if (!scan) return false;
  const b = scan.backend;
  if (a.backend !== b.suggested) return false;
  return b.suggested !== "cuda" || !b.cuda || a.variant.startsWith(`cuda-${b.cuda}`);
}

export function assetLabel(a: Asset) {
  if (a.backend === "cuda") return `CUDA ${a.variant.replace(/^cuda-/, "")}`;
  if (a.backend === "rocm") return `ROCm ${a.variant.replace(/^(rocm|hip)-?/, "")}`.trim();
  return BACKEND[a.backend] ?? a.variant;
}

function assetNote(a: Asset, scan: Scan | null): string {
  if (a.backend === "cpu") return t("models.asset.cpu");
  if (a.backend === "rocm") return scan && !scan.backend.rocm_runtime_found ? t("models.asset.rocm.missing") : t("models.asset.rocm");
  if (a.backend === "cuda") return a.runtime ? t("models.asset.cuda.runtime") : t("models.asset.cuda");
  return t("models.asset.vulkan");
}

type DownloadProps = {
  scan: Scan | null;
  job: EngineDownloadEvent | null;
  installed: EngineRow[];
  onClose: () => void;
  onJob: (j: EngineDownloadEvent | null) => void;
};

function Download({ scan, job, installed, onClose, onJob }: DownloadProps) {
  const [releases, setReleases] = useState<Release[] | null>(null);
  const [tag, setTag] = useState("");
  const [asset, setAsset] = useState("");
  const [error, setError] = useState("");
  const [sending, setSending] = useState(false);

  useEffect(() => {
    api<{ releases: Release[] }>("/system/engine-releases")
      .then((r) => {
        setReleases(r.releases);
        const first = r.releases[0];
        if (first) {
          setTag(first.tag);
          setAsset((first.assets.find((a) => fits(a, scan)) ?? first.assets[0])?.name ?? "");
        }
      })
      .catch((e: unknown) => setError(say(e)));
    // Only once per opening; a scan arriving later does not change a choice already made.
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const running = job !== null && job.phase !== "done" && job.phase !== "failed";
  const release = releases?.find((r) => r.tag === tag);
  const has = (a: Asset) => installed.some((e) => e.origin === "official" && e.version === tag && e.backend === a.backend && e.exe.includes(`${tag}-${a.variant}`));
  const blocked = (a: Asset) => (a.backend === "rocm" && scan !== null && !scan.backend.rocm_runtime_found) || has(a);

  const pickRelease = (t: string) => {
    setTag(t);
    const r = releases?.find((x) => x.tag === t);
    const same = r?.assets.find((a) => a.variant === release?.assets.find((x) => x.name === asset)?.variant);
    setAsset((same ?? r?.assets.find((a) => fits(a, scan)) ?? r?.assets[0])?.name ?? "");
  };

  const start = async () => {
    setSending(true);
    setError("");
    try {
      const r = await postJson<{ job: EngineDownloadEvent }>("/models/engines/download", { tag, asset });
      onJob(r.job);
    } catch (e) {
      setError(say(e));
    } finally {
      setSending(false);
    }
  };

  const chosen = release?.assets.find((a) => a.name === asset);
  return (
    <section className="mp-new pc-dl" aria-label={t("models.engine.get")}>
      <h3>{t("models.engine.get")}</h3>
      {job && <JobView job={job} />}
      {!running && (
        <>
          {releases === null ? (
            !error && (
              <div aria-busy="true">
                <span className="bar-ph" />
              </div>
            )
          ) : releases.length === 0 ? (
            <p className="help">{t("models.engine.noRelease")}</p>
          ) : (
            <>
              <div className="field">
                <span className="field-label" id="pc-rel">
                  {t("models.engine.version")} <small>{t("models.engine.version.note")}</small>
                </span>
                <div className="seg" role="group" aria-labelledby="pc-rel">
                  {releases.map((r) => (
                    <button key={r.tag} type="button" className="num" aria-pressed={r.tag === tag} onClick={() => pickRelease(r.tag)}>
                      {r.tag}
                    </button>
                  ))}
                </div>
              </div>
              <fieldset className="field pc-variants">
                <legend className="field-label">{t("models.engine.variant")}</legend>
                <div className="choices">
                  {release?.assets.map((a) => (
                    <label key={a.name} className={`choice${blocked(a) ? " off" : ""}`}>
                      <input type="radio" name="pc-asset" value={a.name} checked={a.name === asset} disabled={blocked(a)} onChange={() => setAsset(a.name)} />
                      <span className="choice-mark" />
                      <span className="choice-text pc-choice">
                        <span className="pc-choice-head">
                          <b>{assetLabel(a)}</b>
                          {fits(a, scan) && <span className="tag tag-ok">{t("models.engine.fits")}</span>}
                          {has(a) && <span className="tag">{t("models.engine.installed")}</span>}
                        </span>
                        <span className="pc-choice-note">{assetNote(a, scan)}</span>
                        <span className="pc-choice-size num">{mb(a.size + (a.runtime?.size ?? 0))}</span>
                      </span>
                    </label>
                  ))}
                </div>
              </fieldset>
            </>
          )}
          {error && <p className="bad">{error}</p>}
          <div className="btn-row">
            <button type="button" className="btn sm primary" disabled={!chosen || blocked(chosen) || sending} onClick={() => void start()}>
              <Icon name="download" small />
              {sending ? t("models.engine.starting") : chosen ? t("models.tp.get", { size: mb(chosen.size + (chosen.runtime?.size ?? 0)) }) : t("models.get")}
            </button>
            <button
              type="button"
              className="btn sm ghost"
              onClick={() => {
                onJob(null);
                onClose();
              }}
            >
              {t("models.close")}
            </button>
          </div>
        </>
      )}
    </section>
  );
}

export function JobView({ job }: { job: EngineDownloadEvent }) {
  if (job.phase === "failed") {
    return (
      <p className="bad" role="status">
        {t("models.engine.job.failed", {
          what: `${job.tag} ${assetLabel({ backend: job.backend, variant: job.variant } as Asset)}`,
          error: ERRORS[job.error ?? ""] ?? errorText(new ApiError(job.error ?? "unknown")),
        })}
      </p>
    );
  }
  if (job.phase === "done") {
    const devices = job.selftest?.devices ?? [];
    return (
      <p className="pc-done" role="status">
        <Icon name="check" small />
        <span>
          {t("models.engine.job.done", { tag: job.tag, backend: assetLabel({ backend: job.backend, variant: job.variant } as Asset) })}{" "}
          {job.selftest?.ran.length
            ? devices.length
              ? t("models.engine.job.passedOn", { devices: devices.join(", ") })
              : t("models.engine.job.passed")
            : t("models.engine.job.checked")}
        </span>
      </p>
    );
  }
  const at = STEPS.findIndex((s) => s.phase === job.phase);
  const share = job.phase === "download" && job.total > 0 ? job.downloaded / job.total : 1;
  return (
    <div className="pc-job" role="status" aria-label={t("models.engine.job")}>
      <ol className="mp-steps">
        {STEPS.map((s, i) => (
          <li key={s.phase} className={i < at ? "done" : i === at ? "now" : undefined}>
            {i < at ? <Icon name="check" small /> : <i className={`dot ${i === at ? "loading" : "todo"}`} />}
            {s.label}
          </li>
        ))}
      </ol>
      <div className="imp-bar">
        <i style={{ transform: `scaleX(${share})` }} />
      </div>
      <span className="mp-step-text num">
        {job.phase === "download"
          ? t("models.dl.of", { done: mb(job.downloaded), total: mb(job.total) })
          : job.phase === "unpack"
            ? t("models.engine.job.unpack")
            : job.backend === "vulkan"
              ? t("models.engine.job.files")
              : t("models.engine.job.test")}
      </span>
    </div>
  );
}
