// "Handy verbinden" (settings tab "Handy"): the switch, the QR code (decoded, it is exactly the
// phones' address), the pairing code with "Neuer Code", "Alle Handys trennen"; and the page inside the
// Android app (`&shell=android`): "Anderen PC verbinden" and the PC-only settings marked.
//
// The switch opens a second listener. On the test server it must stay on this PC: start the server with
// RP_LAN_BIND=127.0.0.1 and RP_DISCOVERY_PORT=<a free port>, and run the tests with RP_E2E_LAN=1 (they
// skip themselves otherwise, so a run never opens the PC to the network). The switch is off afterwards.

import { mkdirSync } from "node:fs";
import jsQR from "jsqr";
import { api, setTheme, token } from "./api";
import { expect, isPhone, test } from "./fixtures";

const SHOTS = process.env.RP_E2E_SHOTS;
const LAN = process.env.RP_E2E_LAN === "1";

type LanStatus = { enabled: boolean; port: number; url: string | null; code: string | null; ip: string | null; addresses: { ip: string }[] };
const lan = () => api<LanStatus>("GET", "/lan");
const health = async (port: number, t: string) => (await fetch(`http://127.0.0.1:${port}/api/health`, { headers: { Authorization: `Bearer ${t}` } })).status;
const shot = async (page: import("@playwright/test").Page, name: string) => {
  if (!SHOTS) return;
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: `${SHOTS}/${name}-${test.info().project.name}.png`, animations: "disabled" });
};

test.afterAll(async () => {
  if (LAN) await api("PUT", "/lan", { enabled: false });
});

test("Handy verbinden: QR-Code, Code, alle Handys trennen", async ({ page, open }) => {
  test.skip(!LAN, "needs a test server with RP_LAN_BIND=127.0.0.1 and RP_E2E_LAN=1 (see the head of this file)");
  await api("PUT", "/lan", { enabled: false });
  await open("/settings/phone");
  await expect(page.getByRole("heading", { name: "Handy verbinden" })).toBeVisible();
  const toggle = page.getByRole("switch", { name: /Handys dürfen sich verbinden/ });
  await expect(toggle).not.toBeChecked();
  await toggle.click();
  await expect(toggle).toBeChecked();

  const s = await lan();
  expect(s.enabled).toBe(true);
  test.skip(s.addresses.length === 0, "this PC has no network address, so there is no QR code to show");
  const qr = page.getByRole("img", { name: "QR-Code für das Handy" });
  await expect(qr).toBeVisible();
  // The QR code, read back from its pixels, is exactly the phones' address with their token.
  const png = await page.locator(".hv-qr").screenshot();
  const pixels = await page.evaluate(async (b64) => {
    const img = new Image();
    img.src = `data:image/png;base64,${b64}`;
    await img.decode();
    const c = document.createElement("canvas");
    c.width = img.width;
    c.height = img.height;
    const g = c.getContext("2d")!;
    g.drawImage(img, 0, 0);
    return { w: img.width, h: img.height, data: Array.from(g.getImageData(0, 0, img.width, img.height).data) };
  }, png.toString("base64"));
  const read = jsQR(new Uint8ClampedArray(pixels.data), pixels.w, pixels.h);
  expect(read?.data).toBe(s.url);
  expect(s.url).toMatch(new RegExp(`^http://${s.ip!.replace(/\./g, "\\.")}:${s.port}/\\?token=[A-Za-z0-9]{32}$`));
  await expect(page.getByText(`${s.ip}:${s.port}`, { exact: true })).toBeVisible();
  await expect(page.getByText("Handy und PC im selben WLAN; beim ersten Mal fragt Windows nach der Firewall – erlauben.")).toBeVisible();

  // The 6-digit code, and "Neuer Code" replaces it.
  const digits = page.locator("output.hv-digits");
  await expect(digits).toHaveAttribute("data-code", s.code!);
  await expect(digits).toHaveText(`${s.code!.slice(0, 3)} ${s.code!.slice(3)}`);
  await expect(page.getByText(/gültig noch \d+:\d{2}/)).toBeVisible();
  await shot(page, "phone-connect");
  if (isPhone(page)) {
    await page.getByText(`${s.ip}:${s.port}`, { exact: true }).scrollIntoViewIfNeeded();
    await shot(page, "phone-connect-code");
  }
  await page.getByRole("button", { name: "Neuer Code" }).click();
  await expect(digits).not.toHaveAttribute("data-code", s.code!);
  expect((await lan()).code).toBe(await digits.getAttribute("data-code"));

  // The phones' listener answers with their token only.
  const old = new URL(s.url!).searchParams.get("token")!;
  expect(await health(s.port, old)).toBe(200);
  expect(await health(s.port, token)).toBe(401);

  // "Alle Handys trennen": a new token, the old one is refused, this page goes on.
  await page.getByRole("button", { name: "Alle Handys trennen" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Alle Handys sind getrennt" })).toBeVisible();
  const after = await lan();
  expect(after.url).not.toBe(s.url);
  expect(await health(s.port, old)).toBe(401);
  expect(await health(s.port, new URL(after.url!).searchParams.get("token")!)).toBe(200);
  await expect(qr).toHaveAttribute("data-url", after.url!);
  await page.goto(`/?token=${token}#/chars`);
  await expect(page.getByRole("heading", { name: "Charaktere", level: 1 })).toBeVisible();

  // Off: no QR code, the port is closed.
  await page.goto(`/?token=${token}#/settings/phone`);
  await page.getByRole("switch", { name: /Handys dürfen sich verbinden/ }).click();
  await expect(qr).toHaveCount(0);
  await expect.poll(() => health(s.port, new URL(after.url!).searchParams.get("token")!).catch(() => 0)).toBe(0);
});

test("In der Handy-App: anderen PC verbinden, PC-Bereiche markiert", async ({ page, theme }) => {
  await setTheme(theme);
  await page.goto(`/?token=${token}&shell=android#/settings/phone`);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.getByRole("heading", { name: "Verbindung" })).toBeVisible();
  await expect(page.getByRole("switch", { name: /Handys dürfen sich verbinden/ })).toHaveCount(0);
  const other = page.locator("main, .chat-main").getByRole("link", { name: "Anderen PC verbinden" });
  await expect(other).toHaveAttribute("href", "http://tauri.localhost/?rescan=1");
  await shot(page, "phone-app-connection");

  // Engines, model folders, benchmark and restoring backups run at the PC: marked, without their controls.
  await page.goto(`/?token=${token}&shell=android#/settings/expert`);
  await expect(page.getByText("Engines · läuft am PC")).toBeVisible();
  await expect(page.getByText("Modellordner · läuft am PC")).toBeVisible();
  await expect(page.getByRole("button", { name: "Engine herunterladen" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Ordner hinzufügen" })).toHaveCount(0);
  await page.getByText("Engines · läuft am PC").scrollIntoViewIfNeeded();
  await shot(page, "phone-app-expert");
  await page.goto(`/?token=${token}&shell=android#/settings/backups`);
  await expect(page.getByText("Wiederherstellen · läuft am PC")).toBeVisible();
  await expect(page.getByRole("button", { name: "Wiederherstellen" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Jetzt sichern" })).toBeVisible();
  await page.getByText("Wiederherstellen · läuft am PC").scrollIntoViewIfNeeded();
  await shot(page, "phone-app-backups");

  // Ctrl K finds the way to another PC; on a desktop-sized window the sidebar has it too.
  if (!isPhone(page)) {
    await expect(page.getByRole("complementary").getByRole("link", { name: "Anderen PC verbinden" })).toHaveAttribute("href", "http://tauri.localhost/?rescan=1");
  }
  await page.keyboard.press("Control+k");
  await page.keyboard.type("anderen pc");
  await expect(page.getByRole("option", { name: /Anderen PC verbinden/ })).toBeVisible();
  await page.keyboard.press("Escape");

  // The same page on the PC itself (a new tab: the app's mark is kept per tab): no marks.
  const pc = await page.context().newPage();
  await pc.goto(`/?token=${token}#/settings/expert`);
  await expect(pc.getByRole("heading", { name: "Engines" })).toBeVisible();
  await expect(pc.getByText("Engines · läuft am PC")).toHaveCount(0);
  await pc.close();
});
