// "Handy verbinden": the server's switch for phones in the Wi-Fi (`/lan`), and the QR code of its address.

import qrcode from "qrcode-generator";
import { api, postJson } from "../api";

export type LanStatus = {
  enabled: boolean;
  port: number;
  /** This PC's addresses a phone may use, the default first; `adapter` is the network's name in Windows. */
  addresses: { ip: string; adapter: string }[];
  /** The address in the QR code (null: the PC is in no network). */
  ip: string | null;
  /** `http://<ip>:<port>/?token=<token>`, the QR code's content; null while off. */
  url: string | null;
  /** The 6-digit pairing code; null while off. */
  code: string | null;
  code_expires_in: number | null;
  /** Seconds the code is locked after 5 wrong ones. */
  locked_for: number;
  problem: "port_busy" | null;
  /** The phone app can find the PC by itself (UDP search on). */
  search: boolean;
};

export const lanStatus = () => api<LanStatus>("/lan");
export const setLan = (body: { enabled?: boolean; ip?: string }) => postJson<LanStatus>("/lan", body, "PUT");
export const disconnectPhones = () => postJson<LanStatus>("/lan/disconnect", {});
export const newCode = () => postJson<LanStatus>("/lan/code", {});

/** The QR code of `text` as one SVG path in a square of `size` modules (4 modules of quiet zone around). */
export function qrPath(text: string): { size: number; d: string } {
  const qr = qrcode(0, "M");
  qr.addData(text, "Byte");
  qr.make();
  const n = qr.getModuleCount();
  const quiet = 4;
  let d = "";
  for (let r = 0; r < n; r++) {
    for (let c = 0; c < n; c++) {
      if (!qr.isDark(r, c)) continue;
      // Runs of dark modules in a row become one rectangle.
      let end = c;
      while (end + 1 < n && qr.isDark(r, end + 1)) end++;
      d += `M${c + quiet} ${r + quiet}h${end - c + 1}v1h-${end - c + 1}z`;
      c = end;
    }
  }
  return { size: n + quiet * 2, d };
}
