// Streamed text arrives as many small socket messages, each its own task; applied one by one,
// every token would be a render. This collects the text per message and hands it over at most
// once per frame. `flush` hands over at once (before any other event, so the order stays).

export type TokenBuffer = {
  push: (messageId: number, text: string) => void;
  flush: () => void;
};

export function tokenBuffer(apply: (parts: Map<number, string>) => void): TokenBuffer {
  let parts = new Map<number, string>();
  let frame = 0;
  const flush = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    if (parts.size === 0) return;
    const ready = parts;
    parts = new Map();
    apply(ready);
  };
  return {
    push(messageId, text) {
      parts.set(messageId, (parts.get(messageId) ?? "") + text);
      if (!frame) frame = requestAnimationFrame(flush);
    },
    flush,
  };
}
