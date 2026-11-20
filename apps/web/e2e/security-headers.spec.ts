// The page under the server's Content-Security-Policy: it loads and runs without a single console
// error or CSP violation (the inline theme script is allowed by its hash), the token leaves the
// address bar once it is read, and a reload stays connected (token from sessionStorage).

import type { Page } from "@playwright/test";
import { setTheme, token } from "./api";
import { expect, test } from "./fixtures";

/** Console errors, page errors and CSP violations from the moment the page starts. */
function watchProblems(page: Page): string[] {
  const problems: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`page: ${e.message}`));
  return problems;
}

async function violations(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as unknown as { __csp: string[] }).__csp);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const seen: string[] = [];
    (window as unknown as { __csp: string[] }).__csp = seen;
    document.addEventListener("securitypolicyviolation", (e) => seen.push(`${e.violatedDirective} ${e.blockedURI}`));
  });
});

test("Sicherheits-Header: Seite lädt mit CSP ohne Fehler, Token verschwindet aus der Adresse", async ({ page, theme }) => {
  await setTheme(theme);
  const problems = watchProblems(page);
  const res = await page.goto(`/?token=${token}#/chars`);
  const h = res!.headers();
  expect(h["content-security-policy"]).toMatch(/^default-src 'self'; script-src 'self' 'sha256-[A-Za-z0-9+/]+=*';/);
  expect(h["content-security-policy"]).toContain("frame-ancestors 'none'");
  expect(h["referrer-policy"]).toBe("no-referrer");
  expect(h["x-content-type-options"]).toBe("nosniff");

  // The inline theme script ran (it sets data-theme before the app does) and the library loaded.
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
  expect(page.url()).not.toContain("token");
  await expect(page).toHaveURL(/\/#\/chars$/);
  // "Zum Startbildschirm" still gets a start address with the token from the manifest link.
  await expect(page.locator('link[rel="manifest"]')).toHaveAttribute("href", `/manifest.webmanifest?token=${token}`);

  // A reload stays connected and the socket opens under the policy.
  const socket = page.waitForEvent("websocket");
  await page.reload();
  await socket;
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
  expect(page.url()).not.toContain("token");

  expect(await violations(page)).toEqual([]);
  expect(problems).toEqual([]);
});

test("Token aus der Adresse: shell=android und die Route bleiben, Neuladen bleibt in der App-Hülle", async ({ page, theme }) => {
  await setTheme(theme);
  const problems = watchProblems(page);
  await page.goto(`/?token=${token}&shell=android#/settings/phone`);
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  await expect(page).toHaveURL(/\/\?shell=android#\/settings\/phone$/);
  await page.reload();
  // Still the app's view of the connection after the reload (shell kept, token from storage).
  await expect(page.locator("main, .chat-main").getByRole("link", { name: "Anderen PC verbinden" })).toBeVisible();
  await expect(page.getByRole("switch", { name: /Handys dürfen sich verbinden/ })).toHaveCount(0);
  expect(await violations(page)).toEqual([]);
  expect(problems).toEqual([]);
});
