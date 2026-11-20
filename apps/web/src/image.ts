// Taking a picture in for a character or a persona: from a file, a drop, the clipboard or a link.
// Large pictures are scaled down here before they are uploaded, and the crop gets a sensible start.

import { useEffect, useRef, useState, type DragEvent } from "react";
import { ApiError, pictureFromLink } from "./api";
import { t } from "./i18n";

/** Longest side a picture keeps; larger ones are scaled down before the upload. */
export const MAX_SIDE = 2048;

const TYPES = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/** A picture ready to be staged: the file to upload, an object URL to show it, the crop to start with. */
export type Picked = { file: File; url: string; pos: [number, number] };

/** A plain reason for the user; shown next to the picture. */
export class PickError extends Error {}

// Read when the error happens, so they are in the language on screen.
const NOT_IMAGE = () => t("cards.pic.notImage");
const LINK_FAILED = () => t("cards.pic.linkFailed");

/** Where the crop starts: the middle, and for a portrait the top (that is where the face usually is). */
export const startPos = (width: number, height: number): [number, number] => (height > width ? [50, 0] : [50, 50]);

function decode(blob: Blob): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(blob);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      resolve(img);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new PickError(NOT_IMAGE()));
    };
    img.src = url;
  });
}

const extOf: Record<string, string> = { "image/png": "png", "image/jpeg": "jpg", "image/webp": "webp", "image/gif": "gif" };

/** Checks the type, scales a picture larger than `MAX_SIDE` down (same format; a GIF is kept as it is,
 *  it may move) and works out where the crop starts. */
export async function prepare(blob: Blob, name = "bild"): Promise<Picked> {
  const type = blob.type.toLowerCase();
  if (!TYPES.includes(type)) throw new PickError(NOT_IMAGE());
  const img = await decode(blob);
  const w = img.naturalWidth;
  const h = img.naturalHeight;
  if (!w || !h) throw new PickError(NOT_IMAGE());
  let out = blob;
  if (type !== "image/gif" && Math.max(w, h) > MAX_SIDE) {
    const scale = MAX_SIDE / Math.max(w, h);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(w * scale));
    canvas.height = Math.max(1, Math.round(h * scale));
    const g = canvas.getContext("2d");
    if (!g) throw new PickError(NOT_IMAGE());
    g.imageSmoothingQuality = "high";
    g.drawImage(img, 0, 0, canvas.width, canvas.height);
    const smaller = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, 0.92));
    // A browser without an encoder for this format answers PNG; that is fine for the server too.
    if (smaller) out = smaller;
  }
  const base = name.replace(/\.[a-z0-9]+$/i, "") || "bild";
  const file = new File([out], `${base}.${extOf[out.type] ?? "png"}`, { type: out.type });
  return { file, url: URL.createObjectURL(file), pos: startPos(w, h) };
}

/** An address that can point at a picture: http(s) only. */
export function asLink(text: string): string | null {
  const s = text.trim();
  if (!/^https?:\/\/\S+$/i.test(s)) return null;
  try {
    return new URL(s).href;
  } catch {
    return null;
  }
}

/** The app's server fetches the picture (the page may only talk to it, see the CSP) and checks the
 *  address, the size and the bytes. A link it cannot or may not fetch gets the plain advice to save the
 *  picture and drop it; an answer that is no picture says so. */
export async function fromLink(link: string): Promise<Picked> {
  let blob: Blob;
  try {
    blob = await pictureFromLink(link);
  } catch (e) {
    throw new PickError(e instanceof ApiError && e.code === "bad_image" ? NOT_IMAGE() : LINK_FAILED());
  }
  const name = decodeURIComponent(new URL(link).pathname.split("/").pop() || "bild");
  return prepare(blob, name);
}

const editable = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || t instanceof HTMLTextAreaElement || (t instanceof HTMLInputElement && !["checkbox", "radio", "button", "file"].includes(t.type)));

const imageFile = (items: DataTransferItemList | undefined, files: FileList | undefined): File | null => {
  for (const f of Array.from(files ?? [])) if (f.type.startsWith("image/")) return f;
  for (const it of Array.from(items ?? [])) {
    if (it.kind === "file" && it.type.startsWith("image/")) {
      const f = it.getAsFile();
      if (f) return f;
    }
  }
  return null;
};

/** Everything that puts a picture into an editor: a file, a drop on the picture, Ctrl+V while the editor
 *  is open, a link. `take` gets the prepared picture; `error` and `busy` are for the editor to show. */
export function useImageIntake(take: (p: Picked) => void, enabled = true) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [over, setOver] = useState(false);
  // Drag events fire on every child; count them so leaving a child does not end the highlight.
  const depth = useRef(0);
  const takeRef = useRef(take);
  useEffect(() => {
    takeRef.current = take;
  });

  const accept = async (load: () => Promise<Picked>) => {
    setBusy(true);
    setError("");
    try {
      takeRef.current(await load());
    } catch (e) {
      setError(e instanceof PickError ? e.message : NOT_IMAGE());
    } finally {
      setBusy(false);
    }
  };
  const acceptRef = useRef(accept);
  useEffect(() => {
    acceptRef.current = accept;
  });

  // Ctrl+V anywhere while the editor is open: a picture is taken; a link outside a text field too.
  // Text pasted into a field stays ordinary text.
  useEffect(() => {
    if (!enabled) return;
    const onPaste = (e: ClipboardEvent) => {
      const data = e.clipboardData;
      if (!data) return;
      const file = imageFile(data.items, data.files);
      if (file) {
        e.preventDefault();
        void acceptRef.current(() => prepare(file, file.name));
        return;
      }
      if (editable(e.target)) return;
      const link = asLink(data.getData("text/plain"));
      if (link) {
        e.preventDefault();
        void acceptRef.current(() => fromLink(link));
      }
    };
    document.addEventListener("paste", onPaste);
    return () => document.removeEventListener("paste", onPaste);
  }, [enabled]);

  const has = (e: DragEvent) => {
    const types = Array.from(e.dataTransfer.types);
    return types.includes("Files") || types.includes("text/uri-list");
  };
  const dropProps = {
    onDragEnter: (e: DragEvent) => {
      if (!has(e)) return;
      e.preventDefault();
      depth.current += 1;
      setOver(true);
    },
    onDragOver: (e: DragEvent) => {
      if (!has(e)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    },
    onDragLeave: () => {
      depth.current = Math.max(0, depth.current - 1);
      if (depth.current === 0) setOver(false);
    },
    onDrop: (e: DragEvent) => {
      depth.current = 0;
      setOver(false);
      if (!has(e)) return;
      e.preventDefault();
      const file = imageFile(e.dataTransfer.items, e.dataTransfer.files);
      if (file) return void accept(() => prepare(file, file.name));
      // A picture dragged out of another browser window arrives as its address.
      const link = asLink(e.dataTransfer.getData("text/uri-list").split("\n").find((l) => l && !l.startsWith("#")) ?? "");
      if (link) return void accept(() => fromLink(link));
      setError(NOT_IMAGE());
    },
  };

  return {
    error,
    busy,
    over,
    dropProps,
    clearError: () => setError(""),
    fromFile: (file: File) => void accept(() => prepare(file, file.name)),
    fromLink: (link: string) => void accept(() => fromLink(link)),
  };
}
