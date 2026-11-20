// The unsaved draft survives a reload: the texts in localStorage, the picked picture (a file that
// can be several MB) in IndexedDB. Both are per device; nothing goes to the server until "Speichern".

import { emptyDraft, emptyPlan, type Draft, type Plan } from "./draft";

export type Step = "plan" | "build" | "preview";

export type Kept = { plan: Plan; draft: Draft; from: number | null; step: Step };

const KEY = "rp-builder-draft";

export function loadKept(): Kept {
  const fresh: Kept = { plan: emptyPlan(), draft: emptyDraft(), from: null, step: "plan" };
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return fresh;
    const v = JSON.parse(raw) as Partial<Kept>;
    return {
      plan: { ...fresh.plan, ...v.plan },
      draft: { ...fresh.draft, ...v.draft },
      from: typeof v.from === "number" ? v.from : null,
      step: v.step === "build" || v.step === "preview" ? v.step : "plan",
    };
  } catch {
    return fresh;
  }
}

export function storeKept(k: Kept) {
  try {
    localStorage.setItem(KEY, JSON.stringify(k));
  } catch {
    // full or blocked storage: the draft lives on in this window only
  }
}

export function dropKept() {
  try {
    localStorage.removeItem(KEY);
  } catch {
    // nothing stored
  }
}

// ---- the picture

export type KeptPicture = { blob: Blob; name: string; pos: [number, number] };

const DB = "rp-builder";
const STORE = "picture";

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function run<T>(mode: IDBTransactionMode, work: (s: IDBObjectStore) => IDBRequest<T>): Promise<T | undefined> {
  try {
    const db = await open();
    return await new Promise<T>((resolve, reject) => {
      const req = work(db.transaction(STORE, mode).objectStore(STORE));
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    }).finally(() => db.close());
  } catch {
    // no IndexedDB (private window): the picture is simply not kept
    return undefined;
  }
}

export const loadPicture = () => run<KeptPicture | undefined>("readonly", (s) => s.get("draft") as IDBRequest<KeptPicture | undefined>);
export const storePicture = (p: KeptPicture) => run("readwrite", (s) => s.put(p, "draft"));
export const dropPicture = () => run("readwrite", (s) => s.delete("draft"));
