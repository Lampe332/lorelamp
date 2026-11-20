// Direct writes into the test server's database, for data the API cannot create quickly (a chat
// with hundreds of messages would need hundreds of model answers). Only ever the test data folder:
// its path comes from RP_E2E_DATA (see e2e/README.md).

import { DatabaseSync } from "node:sqlite";
import { join } from "node:path";

export const dataDir = process.env.RP_E2E_DATA ?? "";

const LINES = [
  "Der Wind dreht über dem Hafen, und die Möwen ziehen landeinwärts.",
  "Auf dem Tisch liegt eine Seekarte, an den Rändern mit Bleistift beschrieben.",
  "Draußen schlägt die Brandung gegen die Mole, gleichmäßig wie ein Uhrwerk.",
  "Sie dreht den Becher in den Händen und sieht aufs Wasser hinaus.",
  "Im Ofen knackt das Treibholz, und es riecht nach Salz und Rauch.",
];

/** The text of message number `n` (from 2; 1 is the card's greeting): varied lengths, sometimes several paragraphs. */
export function longChatText(n: number): string {
  const sentences = Array.from({ length: 1 + ((n * 7) % 9) }, (_, i) => LINES[(n + i) % LINES.length]);
  const paragraphs: string[] = [];
  for (let i = 0; i < sentences.length; i += 3) paragraphs.push(sentences.slice(i, i + 3).join(" "));
  return `Nachricht ${n}: ${paragraphs.join("\n\n")}`;
}

/**
 * Fills a new chat (which holds only its greeting) up to `total` messages: even numbers are yours,
 * odd ones the character's, each with the text `longChatText(n)`.
 */
export function fillChat(chatId: number, total: number) {
  if (!dataDir) throw new Error("RP_E2E_DATA is not set (the test server's --data-dir, see e2e/README.md)");
  const db = new DatabaseSync(join(dataDir, "rp-app.db"));
  try {
    db.exec("PRAGMA busy_timeout = 5000");
    const have = db.prepare("SELECT COUNT(*) AS n FROM messages WHERE chat_id = ? AND deleted_at IS NULL").get(chatId) as { n: number };
    const add = db.prepare("INSERT INTO messages (chat_id, role, content, created_at) VALUES (?, ?, ?, ?)");
    const t0 = Math.floor(Date.now() / 1000) - total;
    db.exec("BEGIN");
    for (let n = have.n + 1; n <= total; n++) add.run(chatId, n % 2 === 0 ? "user" : "assistant", longChatText(n), t0 + n);
    db.exec("COMMIT");
  } finally {
    db.close();
  }
}

/** Adds these messages to the end of a chat, one minute apart, the last one a minute ago. */
export function addMessages(chatId: number, messages: { role: "user" | "assistant"; content: string }[]) {
  if (!dataDir) throw new Error("RP_E2E_DATA is not set (the test server's --data-dir, see e2e/README.md)");
  const db = new DatabaseSync(join(dataDir, "rp-app.db"));
  try {
    db.exec("PRAGMA busy_timeout = 5000");
    const add = db.prepare("INSERT INTO messages (chat_id, role, content, created_at) VALUES (?, ?, ?, ?)");
    const t0 = Math.floor(Date.now() / 1000) - 60 * messages.length;
    db.exec("BEGIN");
    messages.forEach((m, i) => add.run(chatId, m.role, m.content, t0 + 60 * i));
    db.exec("COMMIT");
  } finally {
    db.close();
  }
}

/** The ids of a chat's active messages, in story order (message number n is at index n - 1). */
export function messageIds(chatId: number): number[] {
  if (!dataDir) throw new Error("RP_E2E_DATA is not set (the test server's --data-dir, see e2e/README.md)");
  const db = new DatabaseSync(join(dataDir, "rp-app.db"));
  try {
    const rows = db.prepare("SELECT id FROM messages WHERE chat_id = ? AND deleted_at IS NULL ORDER BY id").all(chatId) as { id: number }[];
    return rows.map((r) => r.id);
  } finally {
    db.close();
  }
}

/** Stores measurements on a message, as a finished model answer has them. */
export function setTimings(messageId: number, timings: { tok_s: number; processed: number; cached: number; predicted: number }) {
  if (!dataDir) throw new Error("RP_E2E_DATA is not set (the test server's --data-dir, see e2e/README.md)");
  const db = new DatabaseSync(join(dataDir, "rp-app.db"));
  try {
    db.exec("PRAGMA busy_timeout = 5000");
    db.prepare("UPDATE messages SET timings = ? WHERE id = ?").run(JSON.stringify(timings), messageId);
  } finally {
    db.close();
  }
}
