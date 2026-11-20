// A local stand-in for Hugging Face, so the download tests never touch the real site or load a real
// model. It answers the model API (`/api/models/{org}/{name}?blobs=true`, `/api/models?search=`) and
// the file downloads (`/{org}/{name}/resolve/main/{path}`: a redirect to a "CDN" path that honours
// Range with 206), slowly enough to see the progress. The test server must point at it:
// start it with `RP_HF_BASE=http://127.0.0.1:<port>/` (see README); the port is `RP_E2E_HF_PORT`.
//
// It serves tiny files under the names of one catalog entry (the live check then replaces the
// catalog's sizes and hashes with these), plus a test repo for the pasted link.

import { createHash } from "node:crypto";
import { createServer, type Server } from "node:http";

export const HF_PORT = Number(process.env.RP_E2E_HF_PORT ?? 7877);
export const CATALOG_REPO = "mradermacher/Huihui-Qwen3.5-4B-abliterated-GGUF";
/** Under a known quanter's name, so the live search ("Mehr finden") lists it; exists only here. */
export const LINK_REPO = "bartowski/E2E-Tiny-GGUF";

const MB = 1_000_000;

/** Deterministic bytes, so every run (and the live check's cache) sees the same files. */
function bytes(seed: number, len: number): Buffer {
  const b = Buffer.alloc(len);
  let x = seed >>> 0;
  for (let i = 0; i < len; i++) {
    x = (Math.imul(x, 1_664_525) + 1_013_904_223) >>> 0;
    b[i] = x >>> 24;
  }
  return b;
}

const REPOS: Record<string, Record<string, Buffer>> = {
  [CATALOG_REPO]: {
    "Huihui-Qwen3.5-4B-abliterated.IQ4_XS.gguf": bytes(1, 3 * MB),
    "Huihui-Qwen3.5-4B-abliterated.Q4_K_M.gguf": bytes(2, 4 * MB),
    "Huihui-Qwen3.5-4B-abliterated.Q6_K.gguf": bytes(3, 5 * MB),
  },
  [LINK_REPO]: {
    "Tiny-Q3_K_M.gguf": bytes(4, 3 * MB),
    "Tiny-Q4_K_M.gguf": bytes(5, 4 * MB),
    "Tiny-Q6_K.gguf": bytes(6, 5 * MB),
    "mmproj-Tiny-f16.gguf": bytes(7, 1 * MB),
  },
};
const SHA: Record<string, string> = {};
for (const files of Object.values(REPOS)) for (const [p, b] of Object.entries(files)) SHA[p] = createHash("sha256").update(b).digest("hex");

/** Bytes per 100 ms: 1 MB/s. */
const STEP = 100_000;

export function startHfStandIn(port = HF_PORT): Promise<Server> {
  const server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://x");
    const parts = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
    if (parts[0] === "api" && parts[1] === "models" && parts.length === 2) {
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify([{ id: LINK_REPO, downloads: 12, likes: 1, gguf: { total: 1_000_000_000, architecture: "llama", context_length: 8192 } }]));
      return;
    }
    if (parts[0] === "api" && parts[1] === "models" && parts.length === 4) {
      const repo = `${parts[2]}/${parts[3]}`;
      const files = REPOS[repo];
      if (!files) return void res.writeHead(404).end();
      res.setHeader("content-type", "application/json");
      res.end(
        JSON.stringify({
          id: repo,
          gated: false,
          lastModified: "2026-11-01T00:00:00.000Z",
          tags: ["gguf", "license:apache-2.0"],
          cardData: { license: "apache-2.0" },
          siblings: Object.entries(files).map(([p, b]) => ({ rfilename: p, size: b.length, lfs: { sha256: SHA[p], size: b.length, pointerSize: 130 } })),
        }),
      );
      return;
    }
    if (parts.length >= 5 && parts[2] === "resolve") {
      res.writeHead(302, { location: `/cdn/${parts[0]}/${parts[1]}/${parts.slice(4).map(encodeURIComponent).join("/")}` }).end();
      return;
    }
    if (parts[0] === "cdn" && parts.length >= 4) {
      const file = REPOS[`${parts[1]}/${parts[2]}`]?.[parts.slice(3).join("/")];
      if (!file) return void res.writeHead(404).end();
      const m = /^bytes=(\d+)-$/.exec(req.headers.range ?? "");
      const ifRange = req.headers["if-range"];
      const from = m && (!ifRange || ifRange === '"e2e"') ? Number(m[1]) : 0;
      if (from >= file.length && m) return void res.writeHead(416, { "content-range": `bytes */${file.length}` }).end();
      const head: Record<string, string> = { etag: '"e2e"', "accept-ranges": "bytes", "content-length": String(file.length - from) };
      if (from > 0) head["content-range"] = `bytes ${from}-${file.length - 1}/${file.length}`;
      res.writeHead(from > 0 ? 206 : 200, head);
      let at = from;
      const tick = () => {
        if (res.destroyed) return;
        const next = Math.min(file.length, at + STEP);
        res.write(file.subarray(at, next));
        at = next;
        if (at >= file.length) res.end();
        else setTimeout(tick, 100);
      };
      tick();
      return;
    }
    res.writeHead(404).end();
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve(server));
  });
}
