// Settings › Sicherungen: when the last backup was made and "back up now"; the room all backups take
// against their limit, the limit itself (500 MB – 5 GB) and clearing out the automatic ones older than
// some days; the backups grouped by kind with restore, delete and deleting several at once. Every
// delete asks first (with how many go), and so does a smaller limit that removes backups. Copies made
// by hand next to the database can be restored but not deleted.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, ApiError, BACKUP_LIMITS, deleteBackups, errorText, setBackupLimit, type Backup, type BackupOverview } from "../api";
import { logEvent } from "../log";
import { respot } from "../spot";
import { PcOnly } from "../phone/Phone";
import { Icon } from "../ui";
import { dateFormat, numberFormat, raw, t, type Key } from "../i18n";
import { rich } from "../i18n/rich";

const GROUPS: { kind: Backup["kind"]; title: Key; keeps: Key }[] = [
  { kind: "manual", title: "system.bk.group.manual", keeps: "system.bk.group.manual.keeps" },
  { kind: "daily", title: "system.bk.group.daily", keeps: "system.bk.group.daily.keeps" },
  { kind: "weekly", title: "system.bk.group.weekly", keeps: "system.bk.group.weekly.keeps" },
  { kind: "migration", title: "system.bk.group.migration", keeps: "system.bk.group.migration.keeps" },
  { kind: "restore", title: "system.bk.group.restore", keeps: "system.bk.group.restore.keeps" },
];
/** The name of a kind of backup in the language on screen. */
const title = (kind: Backup["kind"]) => raw(GROUPS.find((g) => g.kind === kind)?.title ?? "system.bk.group.manual");
/** The reasons the app removes on its own (by its rule, its limit, or "older than"). */
const AUTO = new Set<Backup["reason"]>(["daily", "migration", "before-restore"]);
const DAY_MS = 86_400_000;
const MAX_DAYS = 3650;

const hm = (ms: number) => dateFormat({ hour: "2-digit", minute: "2-digit" }).format(ms);
const date = (ms: number) => dateFormat({ weekday: "short", day: "numeric", month: "short", year: "numeric" }).format(ms);
const daysAgo = (ms: number) => Math.round((new Date().setHours(0, 0, 0, 0) - new Date(ms).setHours(0, 0, 0, 0)) / 86_400_000);

/** "heute, 08:32", "gestern, 21:10", "vor 3 Tagen", "vor 2 Wochen", "vor 4 Monaten". */
function ago(ms: number) {
  const days = daysAgo(ms);
  if (days <= 0) return t("system.bk.today", { time: hm(ms) });
  if (days === 1) return t("system.bk.yesterday", { time: hm(ms) });
  if (days < 14) return t("system.bk.daysAgo", { n: days });
  if (days < 61) return t("system.bk.weeksAgo", { n: Math.floor(days / 7) });
  return t("system.bk.monthsAgo", { n: Math.floor(days / 30) });
}

/** The exact moment: "Sa., 15. Nov. 2026, 08:32". */
const exact = (ms: number) => t("system.bk.exact", { date: date(ms), time: hm(ms) });

const MB = 1024 * 1024;
const GB = 1024 * MB;
const oneDecimal = (n: number) => numberFormat({ minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(n);
const upToOne = (n: number) => numberFormat({ maximumFractionDigits: 1 }).format(n);
/** "840 KB", "12,4 MB", "1 GB", "412 GB" (units and numbers in the language on screen). */
function bytes(n: number) {
  if (n >= 100 * GB) return t("system.bk.gb", { n: String(Math.round(n / GB)) });
  if (n >= GB) return t("system.bk.gb", { n: upToOne(n / GB) });
  if (n >= MB) return t("system.bk.mb", { n: oneDecimal(n / MB) });
  return t("system.bk.kb", { n: String(Math.max(1, Math.round(n / 1024))) });
}
/** A limit as it is chosen: "500 MB", "1 GB", "5 GB". */
const limitText = (n: number) => (n >= GB ? t("system.bk.gb", { n: upToOne(n / GB) }) : t("system.bk.mb", { n: String(Math.round(n / MB)) }));

/** "1 Sicherung", "3 Sicherungen". */
const count = (n: number) => t("system.bk.count", { n });
const dayText = (n: number) => t("system.bk.days", { n });

/** What a smaller limit removes at once, as the server does it: all that stays above the limit
 *  loses its oldest automatic backups first; the newest one and manual ones always stay. */
function prunedBy(list: Backup[], limit: number) {
  const files = list.filter((b) => b.reason !== "hand").sort((a, b) => b.created_at - a.created_at || (a.name < b.name ? 1 : -1));
  let total = files.reduce((s, b) => s + b.size, 0);
  const out: Backup[] = [];
  for (const f of files.slice(1).reverse()) {
    if (total <= limit) break;
    if (AUTO.has(f.reason)) {
      out.push(f);
      total -= f.size;
    }
  }
  return out;
}

/** The automatic backups made more than `days` days ago (the server's "older than" rule). */
const olderThan = (list: Backup[], days: number) => list.filter((b) => AUTO.has(b.reason) && b.created_at < Date.now() - days * DAY_MS);

const parseDays = (s: string) => (/^\d{1,4}$/.test(s.trim()) && +s >= 1 && +s <= MAX_DAYS ? +s : null);

type Note = { tone: "done" | "full" | "bad"; text: string; at: "save" | "room" | "list" };
type Asking =
  | { what: "restore" | "delete"; b: Backup }
  | { what: "many"; gone: Backup[] }
  | { what: "older"; days: number; gone: Backup[] }
  | { what: "limit"; limit: number; gone: Backup[] };

export function Backups() {
  const [data, setData] = useState<BackupOverview | null>(null);
  const [loadError, setLoadError] = useState("");
  const [note, setNote] = useState<Note | null>(null);
  const [busy, setBusy] = useState<"" | "save" | "restore" | "delete" | "limit">("");
  const [asking, setAsking] = useState<Asking | null>(null);
  const [picking, setPicking] = useState(false);
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [days, setDays] = useState("30");
  const dialog = useRef<HTMLDialogElement>(null);

  const load = useCallback(async () => {
    try {
      const d = await api<BackupOverview>("/backups");
      setData(d);
      setLoadError("");
      // A choice is only kept while its backup is still there.
      setPicked((p) => new Set(d.backups.filter((b) => p.has(b.name)).map((b) => b.name)));
      respot(); // the limit and the clean-up arrive with the list
    } catch (e) {
      setLoadError(errorText(e));
    }
  }, []);
  useEffect(() => void load(), [load]);

  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    if (asking && !d.open) d.showModal();
    if (!asking && d.open) d.close();
  }, [asking]);

  const backupNow = async () => {
    setBusy("save");
    setNote(null);
    try {
      const b = await api<Backup>("/backups", { method: "POST" });
      logEvent("info", "backups", "Sicherung erstellt");
      setNote({ at: "save", tone: "done", text: t("system.bk.saved", { when: ago(b.created_at), size: bytes(b.size) }) });
      await load();
    } catch (e) {
      if (e instanceof ApiError && e.code === "backup_limit") {
        const limit = limitText(data ? data.limit_bytes : GB);
        setNote({
          at: "save",
          tone: "full",
          text: t("system.bk.full", { limit, group: title("manual") }),
        });
      } else {
        setNote({ at: "save", tone: "bad", text: t("system.bk.saveFailed", { error: errorText(e) }) });
      }
    } finally {
      setBusy("");
    }
  };

  const restore = async (b: Backup) => {
    setBusy("restore");
    setNote(null);
    try {
      await api(`/backups/${encodeURIComponent(b.name)}/restore`, { method: "POST" });
      logEvent("info", "backups", "Sicherung wiederhergestellt");
      setNote({ at: "save", tone: "done", text: t("system.bk.restored", { when: ago(b.created_at), group: title("restore") }) });
      await load();
    } catch (e) {
      const why = e instanceof ApiError && e.code === "busy" ? t("system.bk.busy") : errorText(e);
      setNote({ at: "save", tone: "bad", text: t("system.bk.restoreFailed", { error: why }) });
    } finally {
      setBusy("");
      setAsking(null);
    }
  };

  const remove = async (b: Backup) => {
    setBusy("delete");
    setNote(null);
    try {
      await api(`/backups/${encodeURIComponent(b.name)}`, { method: "DELETE" });
      logEvent("info", "backups", "Sicherung gelöscht");
      setNote({ at: "save", tone: "done", text: t("system.bk.deletedOne", { when: exact(b.created_at) }) });
    } catch (e) {
      const why = e instanceof ApiError && e.code === "backup_missing" ? t("system.bk.missing") : errorText(e);
      setNote({ at: "save", tone: "bad", text: t("system.bk.deleteFailed", { error: why }) });
    } finally {
      await load();
      setBusy("");
      setAsking(null);
    }
  };

  /** Deletes the picked backups (all or none) or the automatic ones older than some days. */
  const removeMany = async (a: Extract<Asking, { what: "many" | "older" }>) => {
    const at = a.what === "many" ? "list" : "room";
    setBusy("delete");
    setNote(null);
    try {
      const r = await deleteBackups(a.what === "many" ? { names: a.gone.map((b) => b.name) } : { older_than_days: a.days });
      logEvent("info", "backups", a.what === "many" ? "Mehrere Sicherungen gelöscht" : "Alte Sicherungen gelöscht");
      setNote({ at, tone: "done", text: r.deleted.length > 0 ? t("system.bk.deletedN", { what: count(r.deleted.length) }) : t("system.bk.deletedNone") });
      if (a.what === "many") {
        setPicked(new Set());
        setPicking(false);
      }
    } catch (e) {
      const why = e instanceof ApiError && e.code === "backup_missing" ? t("system.bk.missingOne") : errorText(e);
      setNote({ at, tone: "bad", text: t("system.bk.deleteFailed", { error: why }) });
    } finally {
      await load();
      setBusy("");
      setAsking(null);
    }
  };

  const applyLimit = async (limit: number) => {
    const before = data?.limit_bytes ?? limit;
    setBusy("limit");
    setNote(null);
    try {
      const r = await setBackupLimit(limit);
      logEvent("info", "backups", "Grenze für Sicherungen geändert");
      const tail =
        r.pruned > 0
          ? ` ${t("system.bk.pruned", { n: r.pruned })}`
          : r.limit_bytes < before
            ? ` ${t("system.bk.prunedNone")}`
            : "";
      setNote({ at: "room", tone: "done", text: `${t("system.bk.limitSaved", { limit: limitText(r.limit_bytes) })}${tail}` });
    } catch (e) {
      setNote({ at: "room", tone: "bad", text: t("system.bk.limitFailed", { error: errorText(e) }) });
    } finally {
      await load();
      setBusy("");
      setAsking(null);
    }
  };

  /** A larger limit, or a smaller one that removes nothing, is saved at once; otherwise it asks. */
  const chooseLimit = (limit: number) => {
    if (!data || busy !== "" || limit === data.limit_bytes) return;
    const gone = limit < data.limit_bytes ? prunedBy(data.backups, limit) : [];
    if (gone.length > 0) setAsking({ what: "limit", limit, gone });
    else void applyLimit(limit);
  };

  const askOlder = () => {
    const n = parseDays(days);
    if (!data || n === null) return;
    const gone = olderThan(data.backups, n);
    if (gone.length === 0) {
      setNote({ at: "room", tone: "done", text: t("system.bk.olderNone", { days: dayText(n) }) });
      return;
    }
    setNote(null);
    setAsking({ what: "older", days: n, gone });
  };

  const list = data?.backups ?? null;
  const last = list?.reduce<Backup | null>((a, b) => (a && a.created_at >= b.created_at ? a : b), null) ?? null;
  const deletable = list?.filter((b) => b.reason !== "hand") ?? [];
  const pickedRows = deletable.filter((b) => picked.has(b.name));
  const toggle = (name: string) =>
    setPicked((p) => {
      const next = new Set(p);
      if (!next.delete(name)) next.add(name);
      return next;
    });
  const stopPicking = () => {
    setPicking(false);
    setPicked(new Set());
  };
  const n = parseDays(days);
  const noteAt = (at: Note["at"]) => note?.at === at && <NoteLine note={note} />;

  return (
    <div className="view">
      <div className="page set">
        <div className="set-row">
          <div className="set-label">
            <h2>{t("system.bk.save.title")}</h2>
            <p>{t("system.bk.save.lead")}</p>
          </div>
          <div className="set-panel" data-spot="backup-now">
            <div className="bk-now">
              <div className="bk-last">
                <span className="field-label">{t("system.bk.last")}</span>
                {data === null ? (
                  <span className="bar-ph" aria-busy={!loadError} />
                ) : last ? (
                  <>
                    <b className="num">{ago(last.created_at)}</b>
                    <span className="help">
                      {daysAgo(last.created_at) > 1 ? `${exact(last.created_at)} · ` : ""}
                      {title(last.kind)}
                    </span>
                  </>
                ) : (
                  <b>{t("system.bk.none")}</b>
                )}
              </div>
              <button className="btn primary" onClick={() => void backupNow()} disabled={busy !== ""}>
                <Icon name="download" small />
                {busy === "save" ? t("system.bk.saving") : t("system.bk.saveNow")}
              </button>
            </div>
            <p className="bk-keeps">
              {t("system.bk.keeps")}
            </p>
            {noteAt("save")}
            {loadError && (
              <p className="bad" role="alert">
                {loadError}
              </p>
            )}
          </div>
        </div>
        <div className="set-row">
          <div className="set-label">
            <h2>{t("system.bk.room.title")}</h2>
            <p>{t("system.bk.room.lead")}</p>
          </div>
          <div className="set-panel" data-spot="backup-room">
            {data === null ? (
              !loadError && <p className="empty-line">{t("system.bk.loading")}</p>
            ) : (
              <>
                <Room data={data} />
                <div className="x4-field" data-spot="backup-limit">
                  <span className="field-label" id="x4-limit-label">
                    {t("system.bk.limit")}
                  </span>
                  <div className="x4-seg" role="radiogroup" aria-labelledby="x4-limit-label">
                    {BACKUP_LIMITS.map((l) => (
                      <label className="x4-seg-opt" key={l}>
                        <input
                          type="radio"
                          name="backup-limit"
                          checked={data.limit_bytes === l}
                          onChange={() => chooseLimit(l)}
                          disabled={busy !== ""}
                          data-log-label="Grenze für Sicherungen"
                        />
                        <span className="num">{limitText(l)}</span>
                      </label>
                    ))}
                  </div>
                </div>
                <form
                  className="x4-field x4-older"
                  data-spot="backup-cleanup"
                  onSubmit={(e) => {
                    e.preventDefault();
                    askOlder();
                  }}
                >
                  <label className="field-label" htmlFor="x4-days">
                    {t("system.bk.older")}
                  </label>
                  <div className="x4-older-row">
                    <span className="x4-older-text">
                      {t("system.bk.older.before")}
                      <input
                        id="x4-days"
                        className="input num x4-days"
                        inputMode="numeric"
                        value={days}
                        onChange={(e) => setDays(e.target.value)}
                        aria-invalid={n === null}
                        aria-describedby="x4-older-help"
                      />
                      {t("system.bk.older.after")}
                    </span>
                    <button type="submit" className="btn x4-older-go" disabled={busy !== "" || n === null}>
                      <Icon name="trash" small />
                      {t("system.bk.deleteDots")}
                    </button>
                  </div>
                  <p className="help" id="x4-older-help">
                    {n === null ? t("system.bk.older.bad", { max: String(MAX_DAYS) }) : t("system.bk.older.help")}
                  </p>
                </form>
                {noteAt("room")}
              </>
            )}
          </div>
        </div>
        <div className="set-row">
          <div className="set-label">
            <h2>{t("system.bk.restore")}</h2>
            <p>{t("system.bk.restore.lead")}</p>
          </div>
          <div className="set-panel bk-groups" data-spot="backups">
            <PcOnly what={t("system.bk.restore")}>
              {list === null ? (
                !loadError && <p className="empty-line">{t("system.bk.loading")}</p>
              ) : list.length === 0 ? (
                <div className="bk-empty">
                  <Icon name="memory" />
                  <p>
                    {rich("system.bk.empty", { head: <b>{t("system.bk.empty.head")}</b>, now: t("system.bk.saveNow") })}
                  </p>
                </div>
              ) : (
                <>
                  {deletable.length > 0 && (
                    <div className={`x4-bar${picking ? " on" : ""}`} data-spot="backup-pick">
                      {picking ? (
                        <>
                          <span className="x4-picked num" aria-live="polite">
                            {t("system.bk.picked", { n: pickedRows.length, of: deletable.length })}
                          </span>
                          <span className="x4-bar-acts">
                            <button className="btn sm ghost" onClick={() => setPicked(new Set(deletable.map((b) => b.name)))} disabled={busy !== ""} data-spot-focus>
                              {t("system.bk.all")}
                            </button>
                            <button className="btn sm ghost" onClick={() => setPicked(new Set())} disabled={busy !== "" || pickedRows.length === 0}>
                              {t("system.bk.noneShort")}
                            </button>
                            <button
                              className="btn sm danger"
                              onClick={() => setAsking({ what: "many", gone: pickedRows })}
                              disabled={busy !== "" || pickedRows.length === 0}
                            >
                              <Icon name="trash" small />
                              {t("system.bk.deleteDots")}
                            </button>
                          </span>
                          <button className="btn sm ghost x4-done" onClick={stopPicking} disabled={busy !== ""}>
                            {t("system.bk.done")}
                          </button>
                        </>
                      ) : (
                        <button className="btn sm ghost" onClick={() => setPicking(true)} disabled={busy !== ""} data-spot-focus>
                          <Icon name="check" small />
                          {t("system.bk.deleteSeveral")}
                        </button>
                      )}
                    </div>
                  )}
                  {noteAt("list")}
                  {GROUPS.map((g) => {
                    const rows = list.filter((b) => b.kind === g.kind).sort((a, b) => b.created_at - a.created_at);
                    if (rows.length === 0) return null;
                    return (
                      <section key={g.kind} className="bk-group" aria-labelledby={`bk-g-${g.kind}`}>
                        <div className="bk-group-head">
                          <h3 id={`bk-g-${g.kind}`}>{raw(g.title)}</h3>
                          <span className="help">
                            <span className="num">{rows.length}</span> · {raw(g.keeps)}
                          </span>
                        </div>
                        <ul className={`bk-list${picking ? " x4-picking" : ""}`} aria-label={raw(g.title)}>
                          {rows.map((b) =>
                            picking ? (
                              <PickRow key={b.name} b={b} on={picked.has(b.name)} disabled={busy !== ""} onToggle={() => toggle(b.name)} />
                            ) : (
                              <Row
                                key={b.name}
                                b={b}
                                disabled={busy !== ""}
                                onRestore={() => setAsking({ what: "restore", b })}
                                onDelete={() => setAsking({ what: "delete", b })}
                              />
                            ),
                          )}
                        </ul>
                      </section>
                    );
                  })}
                </>
              )}
            </PcOnly>
          </div>
        </div>
      </div>
      <dialog
        ref={dialog}
        className="bk-dialog"
        aria-labelledby="bk-dialog-title"
        onClose={() => busy === "" && setAsking(null)}
        onCancel={(e) => busy !== "" && e.preventDefault()}
        onMouseDown={(e) => e.target === e.currentTarget && busy === "" && setAsking(null)}
      >
        {asking && question(asking)}
      </dialog>
    </div>
  );

  /** The dialog's question: what goes (or comes back), how many, and that a delete is final. */
  function question(asking: Asking) {
    const cancel = (
      <button className="btn ghost" onClick={() => setAsking(null)} disabled={busy !== ""} autoFocus>
        {t("system.cancel")}
      </button>
    );
    const gone = "gone" in asking ? asking.gone : [];
    const goneBytes = bytes(gone.reduce((s, b) => s + b.size, 0));
    const own = gone.filter((b) => b.kind === "manual").length;
    const deleteBtn = (label: string, run: () => void) => (
      <button className="btn danger" onClick={run} disabled={busy !== ""}>
        <Icon name="trash" small />
        {busy === "delete" || busy === "limit" ? t("system.bk.deleting") : label}
      </button>
    );
    const never = <p>{t("system.bk.never")}</p>;
    switch (asking.what) {
      case "restore":
      case "delete":
        return (
          <div className="bk-dialog-body">
            <h2 id="bk-dialog-title">{asking.what === "restore" ? t("system.bk.ask.restore") : t("system.bk.ask.delete")}</h2>
            <p>
              <b className="num">{exact(asking.b.created_at)}</b> · {title(asking.b.kind)} · <span className="num">{bytes(asking.b.size)}</span>
            </p>
            {asking.what === "restore" ? (
              <p>{t("system.bk.ask.restore.body")}</p>
            ) : (
              <p>{t("system.bk.ask.delete.body")}</p>
            )}
            <div className="btn-row">
              {asking.what === "restore" ? (
                <button className="btn primary" onClick={() => void restore(asking.b)} disabled={busy !== ""}>
                  <Icon name="rewind" small />
                  {busy === "restore" ? t("system.bk.restoring") : t("system.bk.restore")}
                </button>
              ) : (
                deleteBtn(t("system.bk.delete"), () => void remove(asking.b))
              )}
              {cancel}
            </div>
          </div>
        );
      case "many":
        return (
          <div className="bk-dialog-body">
            <h2 id="bk-dialog-title">{t("system.bk.ask.many", { n: gone.length })}</h2>
            <p>
              {own > 0
                ? rich("system.bk.ask.many.own", { size: <b className="num">{goneBytes}</b>, n: own })
                : rich("system.bk.ask.many.body", { size: <b className="num">{goneBytes}</b> })}
            </p>
            {never}
            <div className="btn-row">
              {deleteBtn(t("system.bk.deleteN", { n: gone.length }), () => void removeMany(asking))}
              {cancel}
            </div>
          </div>
        );
      case "older":
        return (
          <div className="bk-dialog-body">
            <h2 id="bk-dialog-title">{t("system.bk.ask.older")}</h2>
            <p>
              {rich("system.bk.ask.older.body", {
                what: <b className="num">{t("system.bk.countAuto", { n: gone.length })}</b>,
                n: gone.length,
                days: dayText(asking.days),
                size: <span className="num">{goneBytes}</span>,
              })}
            </p>
            {never}
            <div className="btn-row">
              {deleteBtn(t("system.bk.deleteN", { n: gone.length }), () => void removeMany(asking))}
              {cancel}
            </div>
          </div>
        );
      case "limit":
        return (
          <div className="bk-dialog-body">
            <h2 id="bk-dialog-title">{t("system.bk.ask.limit", { limit: limitText(asking.limit) })}</h2>
            <p>
              {rich("system.bk.ask.limit.body", {
                what: <b className="num">{t("system.bk.ask.limit.oldest", { n: gone.length })}</b>,
                size: <span className="num">{goneBytes}</span>,
                limit: <span className="num">{limitText(data?.limit_bytes ?? 0)}</span>,
              })}
            </p>
            {never}
            <div className="btn-row">
              {deleteBtn(t("system.bk.ask.limit.go"), () => void applyLimit(asking.limit))}
              {cancel}
            </div>
          </div>
        );
    }
  }
}

/** All backups against their limit, with the free disk space beside it, and a quiet hint from 90 %. */
function Room({ data }: { data: BackupOverview }) {
  const share = data.limit_bytes > 0 ? Math.min(1, data.total_bytes / data.limit_bytes) : 0;
  const used = t("system.bk.used", { used: bytes(data.total_bytes), limit: limitText(data.limit_bytes) });
  return (
    <div className="bk-room">
      <div className="bk-room-head">
        <span className="field-label">{t("system.bk.roomLabel")}</span>
        <span className="bk-room-used num">{used}</span>
      </div>
      <div
        className={`bk-meter${data.near_full ? " high" : ""}`}
        role="meter"
        aria-label={t("system.bk.roomLabel")}
        aria-valuemin={0}
        aria-valuemax={data.limit_bytes}
        aria-valuenow={Math.min(data.total_bytes, data.limit_bytes)}
        aria-valuetext={t("system.bk.usedAria", { used })}
      >
        <i style={{ width: `${share * 100}%` }} hidden={data.total_bytes === 0} />
      </div>
      {data.disk_free !== null && (
        <p className="help">
          {rich("system.bk.diskFree", { free: <span className="num">{bytes(data.disk_free)}</span> })}
        </p>
      )}
      {data.near_full && (
        <p className="x4-near">
          <Icon name="alert" small />
          <span>
            {rich("system.bk.nearFull", { pct: <span className="num">{t("system.pct", { n: Math.min(100, Math.round(share * 100)) })}</span> })}
          </span>
        </p>
      )}
    </div>
  );
}

function NoteLine({ note }: { note: Note }) {
  if (note.tone === "bad")
    return (
      <p className="bad" role="alert">
        {note.text}
      </p>
    );
  return (
    <p className={`bk-note ${note.tone}`} role={note.tone === "full" ? "alert" : "status"}>
      <Icon name={note.tone === "full" ? "alert" : "check"} small />
      <span>{note.text}</span>
    </p>
  );
}

/** What a row says about its backup: when, the date, a copy made by hand, the database version. */
function What({ b }: { b: Backup }) {
  const when = exact(b.created_at);
  return (
    <span className="bk-what">
      <b className="num">{ago(b.created_at)}</b>
      <span>
        {b.reason === "hand" && `${t("system.bk.hand")} · `}
        <span className="bk-date">{daysAgo(b.created_at) > 1 ? when : date(b.created_at)}</span>
        {b.schema !== null && (
          <>
            {" · "}
            <span className="bk-schema">{t("system.bk.schema", { n: String(b.schema) })}</span>
          </>
        )}
      </span>
    </span>
  );
}

function Row({ b, disabled, onRestore, onDelete }: { b: Backup; disabled: boolean; onRestore: () => void; onDelete: () => void }) {
  const when = exact(b.created_at);
  const hand = b.reason === "hand";
  return (
    <li className="bk-row">
      <What b={b} />
      <span className="bk-size num">{bytes(b.size)}</span>
      <button className="btn sm bk-restore" onClick={onRestore} disabled={disabled} aria-label={t("system.bk.restoreOne", { when })}>
        <Icon name="rewind" small />
        {t("system.bk.restore")}
      </button>
      {!hand && (
        <button className="btn sm ghost bk-del" onClick={onDelete} disabled={disabled} aria-label={t("system.bk.deleteOne", { when })} title={t("system.bk.delete")}>
          <Icon name="trash" small />
          <span>{t("system.bk.delete")}</span>
        </button>
      )}
    </li>
  );
}

/** A row while several are picked: the whole row ticks its box; a copy made by hand has none. */
function PickRow({ b, on, disabled, onToggle }: { b: Backup; on: boolean; disabled: boolean; onToggle: () => void }) {
  if (b.reason === "hand")
    return (
      <li className="bk-row x4-pickrow x4-hand">
        <span className="x4-nocheck" />
        <What b={b} />
        <span className="bk-size num">{bytes(b.size)}</span>
      </li>
    );
  return (
    <li className={`bk-row x4-pickrow${on ? " on" : ""}`}>
      <label className="x4-pick" data-log-label="Sicherung auswählen">
        <span className="check">
          <input type="checkbox" checked={on} onChange={onToggle} disabled={disabled} aria-label={t("system.bk.pickOne", { when: exact(b.created_at) })} />
          <Icon name="check" small />
        </span>
        <What b={b} />
        <span className="bk-size num">{bytes(b.size)}</span>
      </label>
    </li>
  );
}
