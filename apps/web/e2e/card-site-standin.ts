// A local stand-in for "some web site with character cards", so the link import's general way is
// tested through the real server without touching the internet. The test server must be allowed to
// fetch from it: start it with `RP_LINK_TEST_ORIGIN=http://127.0.0.1:<port>` and run the tests with
// the same address in `RP_E2E_LINK_SITE` (see README). It serves small SFW cards made here:
//
//   /card/<name>.json   a card file (V2 JSON)
//   /card/<name>.png    the same card as a PNG card (card data in a tEXt chunk)
//   /page/<name>        a page that carries the card in its data and names a picture (og:image)
//   /linked/<name>      a page that links the card's PNG file
//   /img/portrait.png   a picture without card data
//   /blocked            a bot check (403 with `cf-mitigated`)
//   /empty              a page without a card

import { createServer, type Server } from "node:http";
import { fixture } from "./api";

export const LINK_SITE = (process.env.RP_E2E_LINK_SITE ?? "").replace(/\/$/, "");

/** A small SFW card in the V2 format. */
export const cardJson = (name: string) =>
  JSON.stringify({
    spec: "chara_card_v2",
    spec_version: "2.0",
    data: {
      name,
      description: `{{char}} führt die kleine Bergbahn hinauf zur Hütte. Sie kennt jede Weiche und jede Lawinenrinne und hat für jeden Fahrgast einen Rat zum Wetter.`,
      personality: "ruhig, genau, freundlich",
      scenario: "Die letzte Fahrt des Tages, draußen beginnt es zu schneien.",
      first_mes: "*Die Bahn ruckt an.* Setz dich ruhig ans Fenster. Bis zur Hütte sind es zwanzig Minuten.",
      mes_example: "",
      creator: "rp-app-tests",
      tags: ["Berge", "Bahn", "Gemütlich", "Winter", "Alltag"],
    },
  });

// --- PNG cards ---------------------------------------------------------------------------------

const CRC = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(bytes: Buffer): number {
  let c = 0xffffffff;
  for (const b of bytes) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** The chunks of a PNG as `[type, data]`. */
function chunks(png: Buffer): [string, Buffer][] {
  const out: [string, Buffer][] = [];
  for (let at = 8; at + 12 <= png.length; ) {
    const len = png.readUInt32BE(at);
    out.push([png.toString("latin1", at + 4, at + 8), png.subarray(at + 8, at + 8 + len)]);
    at += 12 + len;
  }
  return out;
}

function chunk(type: string, data: Buffer): Buffer {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, "latin1");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), data])), 0);
  return Buffer.concat([head, data, crc]);
}

/** The fixture picture without any card data, optionally with `card` as its card. */
export function png(card?: string): Buffer {
  const source = Buffer.from(fixture("ilse_brandt.png"));
  const parts = [source.subarray(0, 8)];
  for (const [type, data] of chunks(source)) {
    if (type === "tEXt" || type === "iTXt" || type === "zTXt") continue;
    if (type === "IEND" && card) parts.push(chunk("tEXt", Buffer.concat([Buffer.from("chara\0", "latin1"), Buffer.from(Buffer.from(card, "utf8").toString("base64"), "latin1")])));
    parts.push(chunk(type, data));
  }
  return Buffer.concat(parts);
}

// --- The site ----------------------------------------------------------------------------------

const page = (body: string) => `<!doctype html><html lang="de"><head><meta charset="utf-8"><title>Karten</title></head><body>${body}</body></html>`;

/** Starts the stand-in on the port of `RP_E2E_LINK_SITE`; `seen` collects the requested paths. */
export async function startCardSite(): Promise<{ server: Server; seen: string[] }> {
  const seen: string[] = [];
  const server = createServer((req, res) => {
    const path = decodeURIComponent((req.url ?? "/").split("?")[0]);
    seen.push(path);
    const send = (status: number, type: string, body: string | Buffer, headers: Record<string, string> = {}) => {
      res.writeHead(status, { "content-type": type, ...headers });
      res.end(body);
    };
    const [, kind, file = ""] = path.split("/");
    const name = file.replace(/\.(json|png)$/, "");
    if (kind === "card" && file.endsWith(".json")) return send(200, "application/json", cardJson(name));
    if (kind === "card" && file.endsWith(".png")) return send(200, "image/png", png(cardJson(name)));
    if (kind === "img") return send(200, "image/png", png());
    if (kind === "page") {
      const data = JSON.stringify({ props: { pageProps: { character: { card: JSON.parse(cardJson(name)) } } } }).replace(/</g, "\\u003c");
      return send(200, "text/html; charset=utf-8", page(`<meta property="og:image" content="/img/portrait.png"><h1>${name}</h1><script id="__NEXT_DATA__" type="application/json">${data}</script>`));
    }
    if (kind === "linked") return send(200, "text/html; charset=utf-8", page(`<img src="/img/logo.png"><a href="/card/${encodeURIComponent(name)}.png" download>Karte herunterladen</a>`));
    if (kind === "blocked") return send(403, "text/html", "Just a moment...", { "cf-mitigated": "challenge" });
    if (kind === "empty") return send(200, "text/html; charset=utf-8", page("<p>Hier liegt keine Karte.</p>"));
    send(404, "text/plain", "not found");
  });
  const port = Number(new URL(LINK_SITE).port);
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve());
  });
  return { server, seen };
}
