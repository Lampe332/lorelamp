import { isLang, setLanguage, t } from "./i18n";
import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { api, ERROR_TEXT, errorText, modelName, postJson, chatName, type Card, type CharInfo, type ChatInfo, type EngineStatus } from "./api";
import { useFormFactor } from "./app/formfactor";
import { Sidebar, TabBar, TopBar, type Here } from "./app/Sidebar";
import { useRoute } from "./app/route";
import { setModelLoaded } from "./app/engineLoad";
import { useChatSocket } from "./app/useChatSocket";
import { useEngine } from "./app/useEngine";
import { useLibrary } from "./app/useLibrary";
import { ConfirmHost, confirmDelete, confirmRewind } from "./general/confirm";
import { useStartPage } from "./general/start";
import { loadGeneral, useGeneral } from "./general/store";
import { Hub, loadLayouts, type EngineView } from "./Hub";
import { newestTimings } from "./hub/blocks";
import { logEvent, startLogging } from "./log";
import {
  applyTheme,
  DEFAULT_HOME_LAYOUT,
  DEFAULT_THEME,
  isHomeLayout,
  isTheme,
  rememberHomeLayout,
  storedHomeLayout,
  type HomeLayoutId,
  type ThemeId,
} from "./theme";
import { TutorialHost } from "./tutorial/TutorialHost";
import { Icon } from "./ui";

startLogging();

// The side screens come in their own chunks, loaded on first use: the characters list, the character
// page (with the card editor, the user check and the default persona), the app settings with the logs,
// the persona manager and the import page. So do the chat, the quick actions (Ctrl+K), the tablet's
// rail and each start layout (Hub.tsx); the main bundle keeps the shell and the start screen's frame.
const loadChat = () => import("./Chat");
const loadPalette = () => import("./Palette");
const loadRail = () => import("./tablet/Rail");
const loadLibraryPage = () => import("./Library");
const loadCharacter = () => import("./Character");
const loadSettings = () => import("./Settings");
const loadPersonas = () => import("./persona/Personas");
const loadImport = () => import("./import/ImportPage");
const loadBuilder = () => import("./builder/Builder");
const loadCommands = () => import("./commands");
const Chat = lazy(() => loadChat().then((m) => ({ default: m.Chat })));
const Palette = lazy(() => loadPalette().then((m) => ({ default: m.Palette })));
const Rail = lazy(() => loadRail().then((m) => ({ default: m.Rail })));
const Library = lazy(() => loadLibraryPage().then((m) => ({ default: m.Library })));
const CharHeader = lazy(() => loadCharacter().then((m) => ({ default: m.CharHeader })));
const CharChats = lazy(() => loadCharacter().then((m) => ({ default: m.CharChats })));
const CharSettings = lazy(() => loadCharacter().then((m) => ({ default: m.CharSettings })));
const CharTools = lazy(() => loadCharacter().then((m) => ({ default: m.CharTools })));
const ChatCardExport = lazy(() => loadCharacter().then((m) => ({ default: m.ChatCardExport })));
const Settings = lazy(() => loadSettings().then((m) => ({ default: m.Settings })));
const Personas = lazy(() => loadPersonas().then((m) => ({ default: m.Personas })));
const ImportPage = lazy(() => loadImport().then((m) => ({ default: m.ImportPage })));
const Builder = lazy(() => loadBuilder().then((m) => ({ default: m.Builder })));

/** Runs `fn` once the browser has nothing else to do; returns the cancel. */
function whenIdle(fn: () => void): () => void {
  if ("requestIdleCallback" in window) {
    const id = window.requestIdleCallback(fn, { timeout: 3000 });
    return () => window.cancelIdleCallback(id);
  }
  const t = setTimeout(fn, 500);
  return () => clearTimeout(t);
}

/** Fetches the side screens when the browser is idle, so that opening one later does not wait for the
 *  network (route changes are transitions: the old screen stays until it is there). */
function usePrefetchScreens() {
  useEffect(
    () => whenIdle(() => void Promise.all([loadChat(), loadPalette(), loadLibraryPage(), loadCharacter(), loadSettings(), loadPersonas(), loadImport(), loadLayouts()]).catch(() => {})),
    [],
  );
}

const engineLabelOf = (engine: EngineStatus) => t(`shell.engine.${engine}`);

const UNDO_MS = 8000;

type Toast = { text: string; undo?: () => void };

export function App() {
  const route = useRoute();
  const form = useFormFactor();
  const { library, loadError, loadLibrary, loadChats } = useLibrary();
  const { engine, engineNote, info: engineInfo, switching, switchTo, waking, unloaded } = useEngine();
  const [toast, setToast] = useState<Toast | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Card>>({}); // unsaved card edits: "char:3", "chat:12"
  const [editing, setEditing] = useState<CharInfo | null>(null); // whose card the editor has loaded
  const [pendingTheme, setPendingTheme] = useState<ThemeId | null>(null); // chosen here, not yet saved
  const [pendingLayout, setPendingLayout] = useState<HomeLayoutId | null>(null);
  const [palette, setPalette] = useState<string | null>(null); // open quick actions: the text they start with
  const [pendingPersona, setPendingPersona] = useState<number | null>(null); // switched to here, not yet confirmed
  const chatId = route.view === "chat" ? route.chatId : null;
  const toastText = useCallback((text: string) => setToast({ text }), []);
  const { online, busy, chat, setChat, notice, last, isOpen, send } = useChatSocket(chatId, loadLibrary, loadChats, toastText);
  usePrefetchScreens();
  // The general settings (tab "Allgemein"): read once, then kept current by the socket.
  const { undo_secs: undoSecs } = useGeneral().value;
  useEffect(() => void loadGeneral(), []);
  useStartPage(library);
  const [commandKit, setCommandKit] = useState<Awaited<ReturnType<typeof loadCommands>> | null>(null);

  // Phones: the app takes the height that is really visible, so an open keyboard never covers the
  // message field (Safari on iOS keeps the layout height and only shrinks the visual viewport).
  useEffect(() => {
    const vv = window.visualViewport;
    if (!vv) return;
    const fit = () => {
      if (Math.abs(vv.scale - 1) > 0.01) return; // pinch zoom is not a keyboard
      document.documentElement.style.setProperty("--vvh", `${Math.round(vv.height)}px`);
    };
    fit();
    vv.addEventListener("resize", fit);
    return () => vv.removeEventListener("resize", fit);
  }, []);

  // The editor of another card must not lend its header to this one while it loads.
  const editorOf = route.view === "chat-card" ? `chat:${route.chatId}` : route.view === "settings" ? `char:${route.charId}` : "";
  useEffect(() => setEditing(null), [editorOf]);

  useEffect(() => {
    if (!toast) return;
    // A toast with "Rückgängig" stays as long as the settings say.
    const t = window.setTimeout(() => setToast(null), toast.undo ? undoSecs * 1000 : UNDO_MS);
    return () => window.clearTimeout(t);
  }, [toast, undoSecs]);

  /** Opens the latest chat with a character (or its first one); `fresh` starts a new chat. */
  const openChat = async (characterId: number, fresh: boolean) => {
    try {
      const { chat_id } = await postJson<{ chat_id: number }>("/chats", { character_id: characterId, fresh });
      location.hash = `#/chat/${chat_id}`;
    } catch (e) {
      setToast({ text: t("shell.toast.openChatFailed", { error: errorText(e) }) });
    }
  };

  /** "Neuer Chat" on a character's page: the new chat opens at once, as on the start page. */
  const addChat = async (characterId: number) => {
    try {
      const { chat_id } = await postJson<{ chat_id: number }>("/chats", { character_id: characterId, fresh: true });
      location.hash = `#/chat/${chat_id}`;
    } catch (e) {
      setToast({ text: t("shell.toast.newChatFailed", { error: errorText(e) }) });
    }
  };

  const setDraft = (key: string, card: Card | null) =>
    setDrafts((d) => {
      const { [key]: _old, ...rest } = d;
      return card ? { ...rest, [key]: card } : rest;
    });

  /** Soft delete with one step of undo; `what` is "characters" or "chats". */
  const remove = async (what: "characters" | "chats", id: number, label: string) => {
    const ids = what === "chats" ? { chat_id: id } : { character_id: id };
    if (!(await confirmDelete(label))) return false;
    logEvent("info", "chat", what === "chats" ? "Chat löschen" : "Figur löschen", ids);
    try {
      await api(`/${what}/${id}`, { method: "DELETE" });
      setToast({
        text: t("shell.toast.deleted", { what: label }),
        undo: () => {
          logEvent("info", "chat", "Löschen rückgängig", ids);
          setToast(null);
          api(`/${what}/${id}/restore`, { method: "POST" }).catch((e) => setToast({ text: t("shell.toast.restoreFailed", { error: errorText(e) }) }));
        },
      });
      return true;
    } catch (e) {
      setToast({ text: t("shell.toast.deleteFailed", { error: errorText(e) }) });
      return false;
    }
  };

  const editMessage = async (id: number, content: string) => {
    logEvent("info", "chat", "Nachricht bearbeiten", { chat_id: chatId ?? undefined, data: { message_id: id, chars: content.length } });
    try {
      await postJson(`/messages/${id}`, { content }, "PUT");
      return true;
    } catch (e) {
      setToast({ text: t("shell.toast.saveFailed", { error: errorText(e) }) });
      return false;
    }
  };

  /** Removes a message and all after it, with one step of undo. The chat itself arrives over the socket. */
  const rewind = async (id: number) => {
    if (!(await confirmRewind())) return false;
    logEvent("info", "chat", "Zurückspulen", { chat_id: chatId ?? undefined, data: { message_id: id } });
    try {
      const r = await api<{ chat_id: number; removed: number }>(`/messages/${id}/rewind`, { method: "POST" });
      setToast({
        text: t("shell.toast.rewound", { n: r.removed }),
        undo: () => {
          logEvent("info", "chat", "Zurückspulen rückgängig", { chat_id: r.chat_id });
          setToast(null);
          api(`/chats/${r.chat_id}/unrewind`, { method: "POST" }).catch((e) =>
            setToast({ text: t("shell.toast.restoreFailed", { error: errorText(e) }) }),
          );
        },
      });
      return true;
    } catch (e) {
      setToast({ text: t("shell.toast.rewindFailed", { error: errorText(e) }) });
      return false;
    }
  };

  /** Opens a new chat that holds `from` up to the message; `from` stays as it is. */
  const fork = async (id: number, from: ChatInfo) => {
    logEvent("info", "chat", "Abzweigen", { chat_id: from.id, data: { message_id: id } });
    try {
      const title = from.title && `${from.title} (Abzweig)`.slice(0, 80);
      const { chat_id } = await postJson<{ chat_id: number }>(`/messages/${id}/fork`, { title });
      location.hash = `#/chat/${chat_id}`;
      setToast({ text: t("shell.toast.forked") });
    } catch (e) {
      setToast({ text: t("shell.toast.forkFailed", { error: errorText(e) }) });
    }
  };

  const renameChat = async (c: ChatInfo, title: string) => {
    logEvent("info", "chat", "Chat umbenennen", { chat_id: c.id, data: { chars: title.length } });
    try {
      await postJson(`/chats/${c.id}`, { title }, "PUT");
    } catch (e) {
      setToast({ text: t("shell.toast.renameFailed", { error: errorText(e) }) });
    }
  };

  /** Binds one chat to a persona (null: it follows the active one again). Shown here at once; the
   *  server then sends the chat's new snapshot to every device. */
  const bindPersona = async (chatId: number, personaId: number | null) => {
    logEvent("info", "chat", "Chat-Persona gesetzt", { chat_id: chatId, data: { persona_id: personaId } });
    const before = chat?.chatId === chatId ? chat.chat.persona_id : undefined;
    setChat((c) => (c && c.chatId === chatId ? { ...c, chat: { ...c.chat, persona_id: personaId } } : c));
    try {
      await postJson(`/chats/${chatId}/persona`, { persona_id: personaId }, "PUT");
    } catch (e) {
      if (before !== undefined) setChat((c) => (c && c.chatId === chatId ? { ...c, chat: { ...c.chat, persona_id: before } } : c));
      setToast({ text: t("shell.toast.personaNotChanged", { error: errorText(e) }) });
    }
  };

  /** Plays as another persona from now on, on every device; shown here at once. */
  const activatePersona = async (id: number) => {
    logEvent("info", "settings", "Persona gewechselt", { data: { id } });
    setPendingPersona(id);
    try {
      await api(`/personas/${id}/activate`, { method: "PUT" });
      await loadLibrary();
    } catch (e) {
      setToast({ text: t("shell.toast.personaNotSwitched", { error: errorText(e) }) });
    } finally {
      setPendingPersona((p) => (p === id ? null : p));
    }
  };

  // The theme the server keeps; until the library is there, the one index.html set from the last visit stays.
  const theme: ThemeId | null = pendingTheme ?? (library ? (isTheme(library.theme) ? library.theme : DEFAULT_THEME) : null);
  useEffect(() => {
    if (theme) applyTheme(theme);
  }, [theme]);

  // The app's language is the server's (the same on every device); until the library is there, the
  // one index.html and i18n/lang.ts took from the last visit stays. A change draws the app anew.
  useEffect(() => {
    if (library && isLang(library.language)) setLanguage(library.language);
  }, [library]);

  /** Shows the theme at once, then saves it; every other device follows over its socket ("library"). */
  const saveTheme = async (id: ThemeId) => {
    logEvent("info", "settings", "Farbschema geändert", { data: { theme: id } });
    setPendingTheme(id);
    try {
      await postJson("/settings", { theme: id }, "PUT");
      await loadLibrary();
    } catch (e) {
      setToast({ text: t("shell.toast.themeNotSaved", { error: errorText(e) }) });
    } finally {
      setPendingTheme((p) => (p === id ? null : p));
    }
  };

  // The start layout, kept like the theme; this device's last one bridges the moment until the library is there.
  const [storedLayout] = useState(storedHomeLayout);
  const layout: HomeLayoutId =
    pendingLayout ?? (library ? (isHomeLayout(library.home_layout) ? library.home_layout : DEFAULT_HOME_LAYOUT) : (storedLayout ?? DEFAULT_HOME_LAYOUT));
  useEffect(() => {
    if (library) rememberHomeLayout(layout);
  }, [library, layout]);

  const saveLayout = async (id: HomeLayoutId) => {
    logEvent("info", "settings", "Start-Layout geändert", { data: { layout: id } });
    setPendingLayout(id);
    try {
      await postJson("/settings", { home_layout: id }, "PUT");
      await loadLibrary();
    } catch (e) {
      setToast({ text: t("shell.toast.layoutNotSaved", { error: errorText(e) }) });
    } finally {
      setPendingLayout((p) => (p === id ? null : p));
    }
  };

  /** The import page: files, folders, a link, personas. */
  const openImport = () => void (location.hash = "#/import");

  // Ctrl+K (Cmd+K on a Mac): the command line of the start screen when it has one, else the quick actions window.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== "k" || !(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
      e.preventDefault();
      const field = document.querySelector<HTMLInputElement>(".cmd-inline .cmd-input");
      if (field) field.focus();
      else setPalette((p) => (p === null ? "" : null));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The quick actions window comes in its own chunk. What is typed between Ctrl+K and the moment the
  // window is on screen is kept and becomes its starting text, so no letter is lost while it loads.
  const paletteOpen = palette !== null;
  useEffect(() => {
    if (!paletteOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (document.querySelector(".cmd-dialog")) return; // the window is there: its field takes the keys
      if (e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return;
      if (e.key === "Escape") setPalette(null);
      else if (e.key === "Backspace") setPalette((p) => p && p.slice(0, -1));
      else if (e.key.length === 1) setPalette((p) => (p ?? "") + e.key);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [paletteOpen]);

  const activePersona = pendingPersona ?? library?.active_persona_id ?? null;
  const personas = library?.personas ?? [];
  // The quick actions' list comes in its own chunk: at once when it is needed now (the window is open,
  // the start layout "Kommandozentrale" has its command line), else when the browser is idle.
  const needCommands = palette !== null || (route.view === "hub" && layout === "kommando");
  useEffect(() => {
    if (commandKit) return;
    let live = true;
    const load = () => void loadCommands().then((m) => live && setCommandKit(m), () => {});
    const cancel = needCommands ? null : whenIdle(load);
    if (needCommands) load();
    return () => {
      live = false;
      cancel?.();
    };
  }, [commandKit, needCommands]);
  // Unload the model to free the graphics memory, or load it again.
  async function toggleModel(load: boolean) {
    const problem = await setModelLoaded(load);
    if (problem) setToast({ text: problem });
  }
  const commands = !commandKit ? [] : commandKit.buildCommands({
    library,
    theme,
    layout,
    openImport,
    newChat: (id) => void openChat(id, true),
    setTheme: (id) => void saveTheme(id),
    setLayout: (id) => void saveLayout(id),
    activeId: activePersona,
    setPersona: (id) => void activatePersona(id),
    chatId: route.view === "chat" ? route.chatId : null,
    charId: route.view === "chat" ? (chat?.character.id ?? null) : route.view === "settings" || route.view === "chats" ? route.charId : null,
    model: modelName(engineInfo?.model),
    engine: engineInfo?.managed ? { unloaded, setLoaded: (load) => void toggleModel(load) } : null,
  });

  const known = (id: number) => library?.characters.find((c) => c.id === id) ?? null;
  // The library is refreshed on every change, so prefer its entry over the snapshot in a view.
  const freshInfo = (info: CharInfo) => known(info.id) ?? info;
  // The model is not there and the app says why: show that instead of a hopeful "startet".
  const engineLabel = !online
    ? t("shell.engine.connecting")
    : switching
      ? t("shell.engine.switching")
      : unloaded
        ? t("shell.engine.unloaded")
        : waking && engine !== "ready"
          ? t("shell.engine.waking")
          : engine === "offline" && engineNote
            ? t("shell.engine.off")
            : engineLabelOf(engine);
  const shownNote = online && engine !== "ready" ? engineNote : "";
  const go = (hash: string) => () => void (location.hash = hash);
  const missing = () => {
    setToast({ text: ERROR_TEXT.character_missing });
    location.hash = "#/chars";
  };
  // The model as the start screen and the welcome tour show it.
  const engineView: EngineView = {
    status: !online ? "connecting" : unloaded ? "idle" : switching || (engine === "offline" && !engineNote) ? "loading" : engine,
    label: engineLabel,
    info: engineInfo,
  };

  let view;
  if (route.view === "chat") {
    const shown = chat && { ...chat, character: freshInfo(chat.character) };
    view = (
      <Chat
        chat={shown}
        personas={personas}
        activePersonaId={activePersona}
        onPersona={(id) => chat && void bindPersona(chat.chatId, id)}
        engine={engine}
        unloaded={unloaded}
        ctx={engineInfo?.model.ctx ?? null}
        engineNote={online ? engineNote : ""}
        online={online}
        busy={busy}
        notice={notice}
        onSend={(text) => {
          logEvent("info", "chat", "Senden", {
            chat_id: route.chatId,
            data: { chars: text.length, socket: isOpen() },
          });
          send({ type: "send", chat_id: route.chatId, text });
        }}
        onStop={() => {
          logEvent("info", "chat", "Stop", { chat_id: route.chatId });
          send({ type: "stop" });
        }}
        onEdit={editMessage}
        onRewind={rewind}
        onFork={(id) => chat && void fork(id, chat.chat)}
        onBack={go(chat ? `#/char/${chat.character.id}/chats` : "#/chars")}
        unsaved={!!chat && `char:${chat.character.id}` in drafts}
      />
    );
  } else if (route.view === "chat-card") {
    const id = route.chatId;
    const key = `chat:${id}`;
    const header = editing && freshInfo(editing);
    view = (
      <div className="chat-main">
        <CharHeader character={header} tab={null} sub={t("shell.page.chatCard")} backLabel={t("shell.page.backToChat")} onBack={go(`#/chat/${id}`)}>
          {header && <ChatCardExport characterId={header.id} name={header.name} chatId={id} />}
          <a className="btn sm" href={`#/chat/${id}`}>
            <Icon name="chat" small />
            <span className="hide-sm">{t("shell.page.toChat")}</span>
          </a>
        </CharHeader>
        <CharSettings
          key={key}
          source={{ kind: "chat", chatId: id }}
          draft={drafts[key]}
          onDraft={(card) => setDraft(key, card)}
          onLoaded={setEditing}
          onDelete={() => {}}
          onMissing={missing}
        />
      </div>
    );
  } else if (route.view === "settings" || route.view === "chats") {
    const id = route.charId;
    const key = `char:${id}`;
    const header = known(id) ?? (editing?.id === id ? editing : null);
    view = (
      <div className="chat-main">
        <CharHeader
          character={header}
          tab={route.view}
          sub={route.view === "chats" ? t("shell.page.allChats") : t("shell.page.cardTexts")}
          unsaved={key in drafts}
          backLabel={t("shell.page.backToChars")}
          onBack={go("#/chars")}
        >
          {header && <CharTools characterId={header.id} name={header.name} hasPicture={header.has_avatar} onToast={toastText} />}
        </CharHeader>
        {route.view === "settings" ? (
          <CharSettings
            key={key}
            source={{ kind: "character", id }}
            personas={library ? personas : null}
            defaultPersonaId={known(id)?.default_persona_id ?? null}
            live={known(id)}
            activePersonaId={activePersona}
            rev={library}
            draft={drafts[key]}
            onDraft={(card) => setDraft(key, card)}
            onLoaded={setEditing}
            onDelete={(c) => {
              setDraft(key, null);
              void remove("characters", c.id, t("shell.page.quoted", { name: c.name })).then((ok) => ok && (location.hash = "#/chars"));
            }}
            onMissing={missing}
          />
        ) : header ? (
          <CharChats
            character={header}
            rev={library}
            onNew={() => addChat(id)}
            onRename={(c, title) => void renameChat(c, title)}
            onDelete={(c: ChatInfo) => void remove("chats", c.id, `„${chatName(c)}“`)}
          />
        ) : (
          <div className="view">{library && <p className="page empty-line">{t("shell.page.charGone")}</p>}</div>
        )}
      </div>
    );
  } else if (route.view === "app-settings") {
    view = (
      <Settings
        tab={route.tab}
        persona={library ? (personas.find((p) => p.id === activePersona) ?? null) : undefined}
        theme={theme}
        onTheme={(id) => void saveTheme(id)}
        layout={library ? layout : null}
        onLayout={(id) => void saveLayout(id)}
      />
    );
  } else if (route.view === "personas") {
    view = (
      <Personas
        personas={library ? personas : null}
        activeId={activePersona}
        sel={route.sel}
        onActivate={(id) => void activatePersona(id)}
        onChanged={loadLibrary}
        onToast={(text) => setToast({ text })}
      />
    );
  } else if (route.view === "builder") {
    view = <Builder library={library} from={route.from} onToast={(text) => setToast({ text })} />;
  } else if (route.view === "import") {
    view = <ImportPage library={library} />;
  } else if (route.view === "library") {
    view = (
      <Library
        library={library}
        loadError={loadError}
        engineNote={shownNote}
        onRetry={() => void loadLibrary()}
        onChat={(c, fresh) => void openChat(c.id, fresh)}
        onToast={setToast}
      />
    );
  } else {
    view = (
      <Hub
        library={library}
        loadError={loadError}
        engineNote={shownNote}
        onRetry={() => void loadLibrary()}
        layout={layout}
        engine={engineView}
        live={last}
        commands={commands}
        onPalette={(q = "") => setPalette(q)}
        onImport={openImport}
        onNewChat={(id) => void openChat(id, true)}
      />
    );
  }

  const atHub = route.view === "hub";
  const atSettings = route.view === "app-settings";
  const atPersonas = route.view === "personas";
  const here: Here = {
    hub: atHub,
    library: !atHub && !atSettings && !atPersonas && route.view !== "chat" && route.view !== "chat-card",
    // A phone's bottom bar has no place for models: there "Vorschläge" is a settings tab like the others.
    settings: atSettings && (form === "phone" || route.tab !== "suggestions"),
    personas: atPersonas,
    chatId,
    models: form !== "phone" && atSettings && route.tab === "suggestions",
  };
  // The newest answer: measured in this window, else the library's, else what /engine says (the same
  // database value), so start screen and sidebar never disagree.
  const status = { online, engine, engineNote, label: engineLabel, timings: newestTimings(last, library) ?? engineInfo?.last_timings ?? null, info: engineInfo, switching, switchTo, unloaded };
  const persona = { personas, activePersona, onActivate: (id: number) => void activatePersona(id) };
  return (
    // Phones show the two destinations as a bottom bar on the top-level screens only.
    <div className={`app ${atHub || atSettings || route.view === "library" || route.view === "import" || (route.view === "personas" && route.sel === null) ? "with-bar" : "detail"}`}>
      {/* A tablet has its own navigation: a rail in place of the sidebar (see app/formfactor.ts). */}
      {form === "tablet" ? (
        <Suspense fallback={null}>
          <Rail library={library} here={here} status={status} onSearch={() => setPalette("")} {...persona} />
        </Suspense>
      ) : (
        <Sidebar
          library={library}
          here={here}
          status={status}
          onSearch={() => setPalette("")}
          modelLoad={engineInfo?.managed && online && !switching ? { unloaded, onToggle: () => void toggleModel(unloaded) } : null}
          {...persona}
        />
      )}
      <TopBar library={library} status={status} onSearch={() => setPalette("")} {...persona} />

      {/* A side screen opened straight from the address shows an empty page until its chunk is there. */}
      <Suspense fallback={<div className="view" aria-busy="true" />}>{view}</Suspense>

      <TabBar here={here} status={status} onSearch={() => setPalette("")} />

      {toast && (
        <div className="toast" role="status" key={toast.text}>
          <span>{toast.text}</span>
          {toast.undo && (
            <button className="btn sm" onClick={toast.undo}>
              {t("shell.toast.undo")}
            </button>
          )}
          {/* How long the undo stays: a bar that runs out with the toast. */}
          {toast.undo && <i className="toast-timer" style={{ animationDuration: `${undoSecs * 1000}ms` }} aria-hidden="true" />}
        </div>
      )}

      <ConfirmHost />

      <TutorialHost library={library} engine={engineView} />

      {palette !== null && (
        <Suspense fallback={null}>
          <Palette commands={commands} initial={palette} onClose={() => setPalette(null)} />
        </Suspense>
      )}
    </div>
  );
}
