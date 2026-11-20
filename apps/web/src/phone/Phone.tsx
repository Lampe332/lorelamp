// Settings tab "Handy" (`#/settings/phone`). On the PC: the switch for phones in the Wi-Fi, a large QR
// code of the phones' address, the 6-digit code for phones without a camera, the address, the choice
// of network when the PC has several, and "Alle Handys trennen". On another device (a phone, the
// Android app): what this page is connected to and "Anderen PC verbinden".
// Also `PcOnly`: parts of the settings that only make sense at the PC itself.

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { ApiError, apiUrl, copyText, errorText } from "../api";
import { respot } from "../spot";
import { Icon } from "../ui";
import { t } from "../i18n";
import { rich } from "../i18n/rich";
import { disconnectPhones, lanStatus, newCode, qrPath, setLan, type LanStatus } from "./lan";
import { inApp, onOtherDevice, RESCAN_URL } from "./shell";
import "./phone.css";

const say = (e: unknown) =>
  e instanceof ApiError && e.code === "port_busy"
    ? t("system.phone.portBusy")
    : e instanceof ApiError && e.code === "pc_only"
      ? t("system.phone.pcOnly")
      : errorText(e);

export function PhoneTab() {
  return onOtherDevice ? <ThisDevice /> : <ConnectPhones />;
}

/** "123 456": easier to read out and type. */
const grouped = (code: string) => `${code.slice(0, 3)} ${code.slice(3)}`;
const clock = (secs: number) => `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;

function ConnectPhones() {
  const [lan, setStatus] = useState<LanStatus | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState<"" | "switch" | "code" | "cut">("");
  const [note, setNote] = useState("");
  const [copied, setCopied] = useState(false);
  // When the status arrived, so the code's countdown runs between two looks at the server.
  const [arrived, setArrived] = useState(0);
  const [now, setNow] = useState(() => performance.now());

  const take = useCallback((s: LanStatus) => {
    setStatus(s);
    setArrived(performance.now());
    setNow(performance.now());
  }, []);

  const load = useCallback(async () => {
    try {
      take(await lanStatus());
      setError("");
    } catch (e) {
      setError(say(e));
    }
  }, [take]);

  useEffect(() => {
    void load();
  }, [load]);
  const ready = lan !== null;
  useEffect(() => {
    if (ready) respot();
  }, [ready]);

  // While phones may connect: look again every 2 s (a used code is replaced at once), count down every second.
  const on = lan?.enabled ?? false;
  useEffect(() => {
    if (!on) return;
    const poll = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, 2000);
    const tick = window.setInterval(() => setNow(performance.now()), 1000);
    return () => {
      window.clearInterval(poll);
      window.clearInterval(tick);
    };
  }, [on, load]);

  const left = lan?.code_expires_in != null ? Math.max(0, lan.code_expires_in - Math.floor((now - arrived) / 1000)) : null;
  const ranOut = left === 0;
  useEffect(() => {
    if (ranOut) void load();
  }, [ranOut, load]);

  const run = async (what: typeof busy, call: () => Promise<LanStatus>, done = "") => {
    setBusy(what);
    setError("");
    setNote("");
    try {
      take(await call());
      setNote(done);
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy("");
    }
  };

  const copy = async () => {
    if (lan?.url && (await copyText(lan.url))) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    }
  };

  const noNetwork = lan !== null && lan.addresses.length === 0;
  const locked = (lan?.locked_for ?? 0) > 0;
  return (
    <div className="view">
      <div className="page set">
        <div className="set-row">
          <div className="set-label">
            <h2>{t("system.phone.title")}</h2>
            <p>{t("system.phone.lead")}</p>
          </div>
          <div className="set-panel hv-panel" data-spot="phone" aria-busy={lan === null || undefined}>
            <label className={`switch-row${lan === null || busy === "switch" ? " off" : ""}`} data-spot="phone-switch">
              <span className="sw-text">
                <b>{t("system.phone.switch")}</b>
                <span>{t("system.phone.switch.note")}</span>
              </span>
              <input
                type="checkbox"
                role="switch"
                className="switch"
                checked={on}
                disabled={lan === null || busy === "switch"}
                onChange={(e) => void run("switch", () => setLan({ enabled: e.target.checked }))}
              />
            </label>
            <p className="help">
              {t("system.phone.plain")}
            </p>
            {noNetwork && (
              <p className="hv-none" role="status">
                {t("system.phone.noNetwork")}
              </p>
            )}
            {on && lan?.url && lan.ip && (
              <div className="hv-pair">
                <Qr url={lan.url} />
                <div className="hv-side">
                  <div className="hv-code" data-spot="phone-code">
                    <span className="hv-code-label">{t("system.phone.code")}</span>
                    {locked ? (
                      <p className="hv-locked num" role="status">
                        {t("system.phone.locked", { secs: lan.locked_for })}
                      </p>
                    ) : (
                      lan.code && (
                        <output className="hv-digits num" aria-label={t("system.phone.code.aria", { digits: lan.code.split("").join(" ") })} data-code={lan.code}>
                          {grouped(lan.code)}
                        </output>
                      )
                    )}
                    <div className="hv-code-foot">
                      {left !== null && !locked && <span className="hv-left num">{t("system.phone.validFor", { time: clock(left) })}</span>}
                      <button type="button" className="btn sm ghost" disabled={busy !== ""} onClick={() => void run("code", newCode)}>
                        <Icon name="rewind" small />
                        {t("system.phone.newCode")}
                      </button>
                    </div>
                  </div>
                  <p className="hv-addr">
                    {rich("system.phone.address", { address: <span className="num">{`${lan.ip}:${lan.port}`}</span> })}
                  </p>
                  <p className="hv-how">
                    {t("system.phone.how")}
                  </p>
                </div>
              </div>
            )}
            {on && lan && lan.addresses.length > 1 && (
              <div className="field" data-spot="phone-network">
                <span className="field-label" id="hv-net">
                  {t("system.phone.network")} <small>{t("system.phone.network.note")}</small>
                </span>
                <div className="hv-nets" role="group" aria-labelledby="hv-net">
                  {lan.addresses.map((a) => (
                    <button
                      key={a.ip}
                      type="button"
                      className="hv-net"
                      aria-pressed={a.ip === lan.ip}
                      disabled={busy !== ""}
                      onClick={() => void run("switch", () => setLan({ ip: a.ip }))}
                    >
                      <b>{a.adapter || t("system.phone.network")}</b>
                      <span className="num">{a.ip}</span>
                    </button>
                  ))}
                </div>
              </div>
            )}
            {on && lan?.url && (
              <details className="hv-more">
                <summary>{t("system.phone.fullAddress")}</summary>
                <div className="hv-url-row">
                  <code className="hv-url">{lan.url}</code>
                  <button type="button" className="btn sm ghost" onClick={() => void copy()}>
                    <Icon name={copied ? "check" : "copy"} small />
                    {copied ? t("system.phone.copied") : t("system.phone.copy")}
                  </button>
                </div>
                <p className="help">{t("system.phone.homeScreen")}</p>
              </details>
            )}
            {error && (
              <p className="bad" role="alert">
                {error}
              </p>
            )}
          </div>
        </div>
        <div className="set-row">
          <div className="set-label">
            <h2>{t("system.phone.cut")}</h2>
            <p>{t("system.phone.cut.lead")}</p>
          </div>
          <div className="set-panel" data-spot="phone-cut">
            <div className="btn-row">
              <button type="button" className="btn sm" disabled={lan === null || busy !== ""} onClick={() => void run("cut", disconnectPhones, t("system.phone.cut.done"))}>
                <Icon name="close" small />
                {busy === "cut" ? t("system.phone.cutting") : t("system.phone.cut")}
              </button>
            </div>
            {note && (
              <p className="hv-done" role="status">
                <Icon name="check" small />
                {note}
              </p>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/** The QR code: dark modules on light paper in every theme (cameras read it best that way). */
function Qr({ url }: { url: string }) {
  const { size, d } = qrPath(url);
  return (
    <figure className="hv-qr">
      <svg viewBox={`0 0 ${size} ${size}`} role="img" aria-label={t("system.phone.qr")} data-url={url} shapeRendering="crispEdges">
        <rect width={size} height={size} className="hv-qr-paper" />
        <path d={d} className="hv-qr-ink" />
      </svg>
    </figure>
  );
}

/** On a phone or another device: which PC this is, and the way to another one. */
function ThisDevice() {
  const host = new URL(apiUrl(""), location.href).host;
  return (
    <div className="view">
      <div className="page set">
        <div className="set-row">
          <div className="set-label">
            <h2>{t("system.phone.here.title")}</h2>
            <p>{t("system.phone.here.lead")}</p>
          </div>
          <div className="set-panel" data-spot="phone">
            <p className="hv-here">
              {rich("system.phone.here.host", { host: <span className="num">{host}</span> })}
            </p>
            {inApp ? (
              <div className="btn-row">
                <a className="btn sm primary" href={RESCAN_URL} data-log-label="Anderen PC verbinden">
                  <Icon name="qr" small />
                  {t("system.phone.otherPc")}
                </a>
              </div>
            ) : (
              <p className="help">{t("system.phone.otherPc.how")}</p>
            )}
            {!inApp && <p className="help">{t("system.phone.here.homeScreen")}</p>}
          </div>
        </div>
      </div>
    </div>
  );
}

/** Settings that only make sense at the PC itself: on another device a short note takes their place. */
export function PcOnly({ what, children }: { what: string; children: ReactNode }) {
  if (!onOtherDevice) return <>{children}</>;
  return (
    <div className="hv-pc-only" role="note">
      <b>{t("system.phone.pcOnly.head", { what })}</b>
      <span>{t("system.phone.pcOnly.note")}</span>
    </div>
  );
}
