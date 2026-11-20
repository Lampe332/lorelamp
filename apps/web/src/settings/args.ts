// Reading and changing llama-server start options in a profile's load arguments, the way
// llama-server reads them: a flag, then its value (`-c 4096`) or `--flag=value`; the last one counts.
// The options the expert settings show have their own controls; every other argument is kept as it
// is and shown as text ("Weitere Argumente", one option per line).

/** `-c`, `--ctx-size`: a dash and a letter. `-1` and `-.5` are values. */
const isFlag = (t: string) => /^--?[A-Za-z]/.test(t);
const flagName = (t: string) => t.split("=")[0];

/** The value of the last of `names` in `args`; "" for a flag given without a value, null when absent. */
export function argOf(args: string[], names: string[]): string | null {
  let out: string | null = null;
  for (let i = 0; i < args.length; i++) {
    const [flag, inline] = args[i].split(/=(.*)/s);
    if (!isFlag(args[i]) || !names.includes(flag)) continue;
    if (inline !== undefined) out = inline;
    else out = i + 1 < args.length && !isFlag(args[i + 1]) ? args[i + 1] : "";
  }
  return out;
}

/** `args` without every one of `names` (and their values), then `names[0] value` when a value is given. */
export function withArg(args: string[], names: string[], value: string | null): string[] {
  const out: string[] = [];
  let skip = false;
  for (const t of args) {
    if (isFlag(t)) skip = names.includes(flagName(t));
    if (!skip) out.push(t);
  }
  return value === null ? out : [...out, names[0], value];
}

/** The start options with a value, each with every spelling llama-server knows. */
export const FLAGS = {
  ctx: ["-c", "--ctx-size"],
  ngl: ["-ngl", "--gpu-layers", "--n-gpu-layers"],
  ctk: ["-ctk", "--cache-type-k"],
  ctv: ["-ctv", "--cache-type-v"],
  fa: ["-fa", "--flash-attn"],
  batch: ["-b", "--batch-size"],
  ub: ["-ub", "--ubatch-size"],
  ncmoe: ["--n-cpu-moe", "-ncmoe"],
  threads: ["-t", "--threads"],
} as const;

/** The start options without a value: "on" is the first flag, "off" the second (when there is one). */
export const SWITCHES = {
  mmap: ["--mmap", "--no-mmap"],
  mlock: ["--mlock"],
} as const;

export type LoadKey = keyof typeof FLAGS | keyof typeof SWITCHES;
/** One value per option as text ("on"/"off" for a switch); null: not set (llama-server's default). */
export type LoadDraft = Record<LoadKey, string | null>;

const VALUE_KEYS = Object.keys(FLAGS) as (keyof typeof FLAGS)[];
const SWITCH_KEYS = Object.keys(SWITCHES) as (keyof typeof SWITCHES)[];
export const KEYS: LoadKey[] = [...VALUE_KEYS, ...SWITCH_KEYS];

/** Every flag one of the controls covers. */
const COVERED = new Set<string>([...Object.values(FLAGS).flat(), ...Object.values(SWITCHES).flat()]);

function switchOf(args: string[], names: readonly string[]): string | null {
  let out: string | null = null;
  for (const t of args) {
    const i = isFlag(t) ? names.indexOf(flagName(t)) : -1;
    if (i >= 0) out = i === 0 ? "on" : "off";
  }
  return out;
}

export function draftOf(args: string[]): LoadDraft {
  const out = {} as LoadDraft;
  for (const k of VALUE_KEYS) out[k] = argOf(args, [...FLAGS[k]]);
  for (const k of SWITCH_KEYS) out[k] = switchOf(args, SWITCHES[k]);
  return out;
}

/** The arguments with the draft's values; options the draft does not change keep their place. */
export function applyDraft(args: string[], from: LoadDraft, to: LoadDraft): string[] {
  let out = args;
  for (const k of VALUE_KEYS) if (from[k] !== to[k]) out = withArg(out, [...FLAGS[k]], to[k] === "" ? null : to[k]);
  for (const k of SWITCH_KEYS) {
    if (from[k] === to[k]) continue;
    const names = SWITCHES[k] as readonly string[];
    out = withArg(out, [...names], null);
    const flag = to[k] === "on" ? names[0] : to[k] === "off" ? names[1] : undefined;
    if (flag) out = [...out, flag];
  }
  return out;
}

/** The arguments split into the ones the controls cover and the rest, both in their order. */
export function splitArgs(args: string[]): { covered: string[]; rest: string[] } {
  const covered: string[] = [];
  const rest: string[] = [];
  let mine = false;
  for (const t of args) {
    if (isFlag(t)) mine = COVERED.has(flagName(t));
    (mine ? covered : rest).push(t);
  }
  return { covered, rest };
}

/** The rest as text: a flag and its first value on one line, any further value on a line of its own. */
export function restText(rest: string[]): string {
  const lines: string[] = [];
  let open = false; // the last line is a flag still waiting for its value
  for (const t of rest) {
    if (open && !isFlag(t)) {
      lines[lines.length - 1] += ` ${t}`;
      open = false;
    } else {
      lines.push(t);
      open = isFlag(t) && !t.includes("=");
    }
  }
  return lines.join("\n");
}

/** The text back as arguments: a line "-np 1" is a flag and its value (the value may hold spaces). */
export function parseRest(text: string): string[] {
  const out: string[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const m = /^(--?[A-Za-z][^\s=]*)\s+(.+)$/.exec(line);
    if (m) out.push(m[1], m[2].trim());
    else out.push(line);
  }
  return out;
}

/** Flags the app sets itself; the server refuses them in a profile. */
export const MANAGED = ["-m", "--model", "--host", "--port", "-hf", "--hf-repo"];

/** The first managed flag in `args`, if any. */
export const managedIn = (args: string[]) => args.find((t) => isFlag(t) && MANAGED.includes(flagName(t)));

/** The arguments of a profile with the controls' draft; `rest`: the other arguments as typed (null: unchanged). */
export function buildArgs(args: string[], from: LoadDraft, to: LoadDraft, rest: string[] | null): string[] {
  return applyDraft(rest === null ? args : [...splitArgs(args).covered, ...rest], from, to);
}
