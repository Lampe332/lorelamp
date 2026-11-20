// Direct calls to the test server's HTTP API, used to put test data and settings in place.
// URL and token come from the environment (see e2e/README.md); nothing here is machine-specific.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const baseURL = (process.env.RP_E2E_URL ?? "http://127.0.0.1:7861").replace(/\/$/, "");
/** A test that restarts the test server (a new token at each start) leaves the new token here for
 *  the later workers of the same run; the run's id comes from global setup. */
const tokenFile = process.env.RP_E2E_RUN ? join(tmpdir(), `rp-e2e-token-${process.env.RP_E2E_RUN}.txt`) : "";
export let token = (tokenFile && existsSync(tokenFile) ? readFileSync(tokenFile, "utf8").trim() : "") || (process.env.RP_E2E_TOKEN ?? "");

export function setToken(next: string) {
  token = next;
  if (tokenFile) writeFileSync(tokenFile, next);
}

export async function api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
  const raw = body instanceof Uint8Array;
  const res = await fetch(`${baseURL}/api${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${token}`,
      ...(body === undefined || raw ? {} : { "Content-Type": "application/json" }),
    },
    body: body === undefined ? undefined : raw ? (body as BodyInit) : JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${method} ${path} -> ${res.status} ${await res.text()}`);
  const text = await res.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

export type Library = {
  characters: { id: number; name: string; chats: number }[];
  personas: { id: number; name: string }[];
  active_persona_id: number;
  theme: string;
  home_layout: string;
};

export const library = () => api<Library>("GET", "/library");

export const fixture = (name: string) =>
  new Uint8Array(readFileSync(fileURLToPath(new URL(`../../../tests/fixtures/${name}`, import.meta.url))));

/** Theme and start layout are server settings; the page mirrors them on load. */
export const setTheme = (theme: string) => api("PUT", "/settings", { theme });
export const setLayout = (home_layout: string) => api("PUT", "/settings", { home_layout });

export async function charId(name: string): Promise<number> {
  const c = (await library()).characters.find((c) => c.name === name);
  if (!c) throw new Error(`test character ${name} missing; did global setup run?`);
  return c.id;
}

export async function newChat(characterId: number): Promise<number> {
  const chat = await api<{ chat_id: number }>("POST", "/chats", { character_id: characterId, fresh: true });
  return chat.chat_id;
}

export const deleteChat = (id: number) => api("DELETE", `/chats/${id}`);
