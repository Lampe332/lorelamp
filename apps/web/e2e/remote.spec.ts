// The UI delivered from somewhere else than the server (later: the phone app). A second, plain static
// server on a free port hands out the same built UI under another origin; the page is pointed at the
// test server only through the stored server address + token (`connectTo` in src/api.ts), not through
// its own address. The library must load over HTTP and the chat over the socket.

import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import type { Page } from "@playwright/test";
import { api, baseURL, setTheme, token } from "./api";
import { expect, test } from "./fixtures";

let ui: Server;
let uiOrigin = "";

test.beforeAll(async () => {
  // Static files only: every path is fetched once from the test server (which serves the built UI and
  // falls back to index.html) and handed out unchanged under this server's own origin.
  ui = createServer((req, res) => {
    fetch(`${baseURL}${req.url ?? "/"}`)
      .then(async (r) => {
        res.writeHead(r.status, { "Content-Type": r.headers.get("content-type") ?? "application/octet-stream" });
        res.end(Buffer.from(await r.arrayBuffer()));
      })
      .catch(() => {
        res.writeHead(502);
        res.end();
      });
  });
  await new Promise<void>((done) => ui.listen(0, "127.0.0.1", done));
  // "localhost" instead of the test server's "127.0.0.1": a different host and port, so a different origin.
  uiOrigin = `http://localhost:${(ui.address() as AddressInfo).port}`;
});

test.afterAll(async () => {
  await new Promise((done) => ui.close(done));
});

/** Whether the server lets a page of another origin read its answers (CORS preflight for the API). */
async function serverAllows(origin: string): Promise<boolean> {
  const res = await fetch(`${baseURL}/api/library`, {
    method: "OPTIONS",
    headers: { Origin: origin, "Access-Control-Request-Method": "GET", "Access-Control-Request-Headers": "authorization,x-rp-client" },
  });
  return res.headers.get("access-control-allow-origin") === origin || res.headers.get("access-control-allow-origin") === "*";
}

type Recent = { chat: { id: number }; name: string };

/** Opens the UI under the other origin, pointed at the test server by the stored address + token
 *  (no token in the page address). Returns the origins of the sockets the page opens. */
async function openRemote(page: Page, hash: string): Promise<string[]> {
  await page.addInitScript(
    ([key, value, server]) => {
      if (location.origin !== server) localStorage.setItem(key, value);
    },
    ["rp-server", JSON.stringify({ base: baseURL, token }), new URL(baseURL).origin] as const,
  );
  const sockets: string[] = [];
  page.on("websocket", (ws) => sockets.push(new URL(ws.url()).origin));
  await page.goto(`${uiOrigin}/#${hash}`);
  return sockets;
}

const newest = async () => (await api<{ recent: Recent[] }>("GET", "/library")).recent[0];

test("Fremder Ursprung: Chat-Socket verbindet mit der eingestellten Server-Adresse", async ({ page }) => {
  const recent = await newest();
  const sockets = await openRemote(page, `/chat/${recent.chat.id}`);
  // The chat's history only comes over the socket.
  await expect(page.getByRole("heading", { name: recent.name, level: 1 })).toBeVisible();
  await expect(page.getByRole("article").first()).toBeVisible();
  expect(new Set(sockets)).toEqual(new Set([new URL(baseURL).origin.replace(/^http/, "ws")]));
});

test("Fremder Ursprung: Bibliothek lädt über HTTP mit dem eingestellten Token", async ({ page, theme }) => {
  // The UI side is in place; the server side (CORS for /api) is not yet: see docs/rework.md, "R3b rest + R4 UI".
  test.skip(!(await serverAllows(uiOrigin)), "server sends no CORS headers for other origins yet");
  await setTheme(theme);
  const recent = await newest();
  await openRemote(page, "/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.getByRole("link", { name: new RegExp(recent.name) }).first()).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "Modell bereit" }).first()).toBeAttached();
});
