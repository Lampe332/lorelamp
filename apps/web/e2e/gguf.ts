// Tiny GGUF files for the model list tests: a valid header (name, architecture, quant, context,
// layers) and one tensor of filler bytes. Nothing a model server could run; only "Meine Modelle"
// reads them.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

type Mini = { name: string; arch?: string; ctx?: number; layers?: number; bytes?: number };

function str(s: string): Buffer {
  const b = Buffer.from(s, "utf8");
  const len = Buffer.alloc(8);
  len.writeBigUInt64LE(BigInt(b.length));
  return Buffer.concat([len, b]);
}
const u32 = (n: number) => {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n);
  return b;
};
const u64 = (n: number) => {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(BigInt(n));
  return b;
};

export function miniGguf({ name, arch = "llama", ctx = 8192, layers = 4, bytes = 4096 }: Mini): Buffer {
  const kv = [
    Buffer.concat([str("general.architecture"), u32(8), str(arch)]),
    Buffer.concat([str("general.name"), u32(8), str(name)]),
    Buffer.concat([str("general.file_type"), u32(4), u32(15)]), // Q4_K_M
    Buffer.concat([str(`${arch}.context_length`), u32(4), u32(ctx)]),
    Buffer.concat([str(`${arch}.block_count`), u32(4), u32(layers)]),
  ];
  const tensor = Buffer.concat([str("token_embd.weight"), u32(1), u64(bytes), u32(0), u64(0)]);
  let head = Buffer.concat([Buffer.from("GGUF"), u32(3), u64(1), u64(kv.length), ...kv, tensor]);
  head = Buffer.concat([head, Buffer.alloc((32 - (head.length % 32)) % 32)]);
  return Buffer.concat([head, Buffer.alloc(bytes, 7)]);
}

export function writeMiniGguf(path: string, mini: Mini) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, miniGguf(mini));
}
