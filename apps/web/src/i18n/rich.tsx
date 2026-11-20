// A sentence with markup in it: `rich("shell.general.persona.macro", { macro: <code>…</code>, name })`
// puts each node where the sentence of the language has its `{place}`, so the word order is the
// translator's.

import { Fragment, type ReactNode } from "react";
import { raw, type Key, type Params } from "./index";

export function rich<K extends Key>(key: K, slots: { [P in keyof Params<K>]: ReactNode }): ReactNode {
  const all = slots as Record<string, ReactNode>;
  const n = typeof all.n === "number" ? all.n : undefined;
  return raw(key, n)
    .split(/(\{\w+\})/)
    .map((part, i) => {
      const name = /^\{(\w+)\}$/.exec(part)?.[1];
      return <Fragment key={i}>{name !== undefined && name in all ? all[name] : part}</Fragment>;
    });
}
