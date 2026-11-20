// A local stand-in for the llama.cpp releases on GitHub, so "Engine einrichten" never downloads from
// the real site. It answers the release list (`/repos/ggml-org/llama.cpp/releases`) with one release
// that has a CPU build, and serves that build as a zip at `/dl/<name>`. The zip holds a real official
// CPU build already on this PC (the folder in `RP_E2E_LLAMA_CPU`, e.g. the one the tests' CPU model
// runs with), packed here, so the server's self-test (`--list-devices`, `--version`, CPU only) passes.
// The test server must point at it: `RP_GITHUB_API=http://127.0.0.1:<port>` (port `RP_E2E_GH_PORT`).

import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import { join } from "node:path";
import { crc32 } from "node:zlib";

export const GH_PORT = Number(process.env.RP_E2E_GH_PORT ?? 7879);
export const CPU_DIR = process.env.RP_E2E_LLAMA_CPU ?? "";
export const TAG = "b99001";
export const ASSET = `llama-${TAG}-bin-win-cpu-x64.zip`;

/** A zip without compression (method 0) of `files`: enough for the server's unpacker. */
function storedZip(files: { name: string; data: Buffer }[]): Buffer {
  const parts: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const f of files) {
    const name = Buffer.from(f.name, "utf8");
    const crc = crc32(f.data) >>> 0;
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0, 6);
    local.writeUInt16LE(0, 8);
    local.writeUInt16LE(0, 10);
    local.writeUInt16LE(0x21, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(f.data.length, 18);
    local.writeUInt32LE(f.data.length, 22);
    local.writeUInt16LE(name.length, 26);
    local.writeUInt16LE(0, 28);
    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(0x02014b50, 0);
    dir.writeUInt16LE(20, 4);
    dir.writeUInt16LE(20, 6);
    dir.writeUInt16LE(0, 8);
    dir.writeUInt16LE(0, 10);
    dir.writeUInt16LE(0, 12);
    dir.writeUInt16LE(0x21, 14);
    dir.writeUInt32LE(crc, 16);
    dir.writeUInt32LE(f.data.length, 20);
    dir.writeUInt32LE(f.data.length, 24);
    dir.writeUInt16LE(name.length, 28);
    dir.writeUInt32LE(offset, 42);
    central.push(dir, name);
    parts.push(local, name, f.data);
    offset += local.length + name.length + f.data.length;
  }
  const dirBytes = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(dirBytes.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, dirBytes, end]);
}

/** Starts the stand-in; resolves to a stop function. Needs `RP_E2E_LLAMA_CPU`. */
export async function startGithubStandin(): Promise<() => Promise<void>> {
  const names = readdirSync(CPU_DIR).filter((n) => n === "llama-server.exe" || n.toLowerCase().endsWith(".dll"));
  if (!names.includes("llama-server.exe")) throw new Error(`no llama-server.exe in RP_E2E_LLAMA_CPU (${CPU_DIR})`);
  const zip = storedZip(names.map((name) => ({ name, data: readFileSync(join(CPU_DIR, name)) })));
  const sha = createHash("sha256").update(zip).digest("hex");
  const base = `http://127.0.0.1:${GH_PORT}`;
  const releases = [
    {
      tag_name: TAG,
      draft: false,
      published_at: "2026-11-01T12:00:00Z",
      assets: [{ name: ASSET, size: zip.length, digest: `sha256:${sha}`, browser_download_url: `${base}/dl/${ASSET}` }],
    },
  ];
  const server: Server = createServer((req, res) => {
    const path = (req.url ?? "").split("?")[0];
    if (path === "/repos/ggml-org/llama.cpp/releases") {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify(releases));
    } else if (path === `/dl/${ASSET}`) {
      res.writeHead(200, { "content-type": "application/zip", "content-length": zip.length });
      res.end(zip);
    } else {
      res.writeHead(404).end();
    }
  });
  await new Promise<void>((ok) => server.listen(GH_PORT, "127.0.0.1", ok));
  return () => new Promise<void>((ok) => server.close(() => ok()));
}
