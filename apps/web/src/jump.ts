// "Open this chat at that message": a search hit (Ctrl+K, group "In Chats") asks for it, the chat
// view takes it once the chat with that message is on screen, scrolls there and highlights it.

type Jump = { chatId: number; messageId: number };

let pending: Jump | null = null;
const EVENT = "rp-jump";

/** Opens the chat and asks it to show the message. */
export function jumpToMessage(chatId: number, messageId: number) {
  pending = { chatId, messageId };
  if (location.hash !== `#/chat/${chatId}`) location.hash = `#/chat/${chatId}`;
  window.dispatchEvent(new Event(EVENT));
}

/** The message to show in this chat, if one was asked for; taken only once it is there. */
export function takeJump(chatId: number, has: (messageId: number) => boolean): number | null {
  if (!pending || pending.chatId !== chatId || !has(pending.messageId)) return null;
  const id = pending.messageId;
  pending = null;
  return id;
}

/** Calls `fn` whenever a jump is asked for. */
export function onJump(fn: () => void): () => void {
  window.addEventListener(EVENT, fn);
  return () => window.removeEventListener(EVENT, fn);
}

/** Opens the search bar of the chat on screen (Ctrl+K "Im Chat suchen"). */
export const CHAT_FIND_EVENT = "rp-chat-find";
