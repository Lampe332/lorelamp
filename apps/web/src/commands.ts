// The commands of the quick actions (Ctrl+K) and of the command line of the "Kommandozentrale" start
// layout: characters, chats, pages and settings, each with the words that find it. Loaded in its own
// chunk right after the start (App.tsx), so the list is not part of the main bundle.

import { lang, LANGS, t } from "./i18n";
import { snippet, when, type Library } from "./api";
import { general } from "./general/store";
import { CHAT_FIND_EVENT } from "./jump";
import { LIBRARY_LAYOUTS, requestLibraryLayout } from "./library/layout";
import { CHAT_PERSONA_EVENT } from "./persona/common";
import { notifyPrefs } from "./notify";
import { inApp, onOtherDevice, RESCAN_URL } from "./phone/shell";
import { openSetting } from "./spot";
import { HOME_LAYOUTS, THEMES, type HomeLayoutId, type ThemeId } from "./theme";
import { restartTour } from "./tutorial/store";
import type { IconName, Portrait } from "./ui";

export type Command = {
  id: string;
  group: string;
  label: string;
  /** Second line or right-hand note. */
  hint?: string;
  /** Where the action leads, shown at the right ("Einstellungen › Darstellung"). */
  where?: string;
  icon?: IconName;
  /** A character's picture instead of an icon. */
  who?: Parameters<typeof Portrait>[0]["who"];
  /** `who` is a persona, not a character. */
  whoKind?: "persona";
  /** A small sample of a colour theme instead of an icon. */
  swatch?: ThemeId;
  /** Marked as the current choice. */
  current?: boolean;
  /** Other words that find this: synonyms and rough descriptions. */
  keywords?: string;
  /** Words that find this only weakly (it comes after better matches). */
  loose?: string;
  /** Position in the list shown before anything is typed. */
  idle?: number;
  /** Small lift in the ranking (settings over look-alike actions). */
  boost?: number;
  /** What the log records (never a name). */
  kind: string;
  /** Instead of running: puts this text into the field, a second step (e.g. "Farbschema "). */
  fills?: string;
  run?: () => void;
};

export type CommandDeps = {
  library: Library | null;
  theme: ThemeId | null;
  layout: HomeLayoutId;
  /** Opens the import page. */
  openImport: () => void;
  newChat: (characterId: number) => void;
  setTheme: (id: ThemeId) => void;
  setLayout: (id: HomeLayoutId) => void;
  /** The persona the user plays as now (null until the library is there). */
  activeId: number | null;
  setPersona: (id: number) => void;
  /** The chat on screen, if any: its persona can be chosen from here. */
  chatId: number | null;
  /** The character on screen (its chat, chats or card), if any: its default persona comes first. */
  charId: number | null;
  /** The name of the model that runs, if known. */
  model: string | null;
  /** Unloading the model (only when this app runs it). */
  engine?: { unloaded: boolean; setLoaded: (load: boolean) => void } | null;
};

const go = (hash: string) => () => void (location.hash = hash);

export function buildCommands({ library, theme, layout, openImport, newChat, setTheme, setLayout, activeId, setPersona, chatId, charId, model, engine }: CommandDeps): Command[] {
  const out: Command[] = [];
  // Words shared by several commands, in the language on screen.
  const personaWords = t("cmd.words.persona");
  const modelWords = t("cmd.words.model");
  const fitWords = t("cmd.words.fit");
  const recent = library?.recent ?? [];
  const chars = library?.characters ?? [];
  const lead = recent[0];
  const themeName = THEMES.find((th) => th.id === theme)?.name;
  const layoutName = HOME_LAYOUTS.find((l) => l.id === layout)?.name;
  const personas = library?.personas ?? [];
  const active = personas.find((p) => p.id === activeId);
  if (lead) {
    out.push({
      id: "resume",
      group: "Weiter",
      label: t("cmd.resume", { name: lead.name }),
      hint: lead.chat.title || when(lead.chat.updated_at),
      icon: "next",
      keywords: t("cmd.resume.keys"),
      idle: 1,
      kind: "weiterschreiben",
      run: go(`#/chat/${lead.chat.id}`),
    });
  }
  if (chars.length > 0) {
    out.push({ id: "new", group: "Aktionen", label: t("cmd.new"), hint: t("cmd.new.hint"), icon: "plus", keywords: t("cmd.new.keys"), idle: 5, kind: "neuer-chat-waehlen", fills: t("cmd.newWith", { name: "" }) });
  }
  if (engine) {
    // The graphics memory: unload the model, or load it again (the next message would, too).
    const unload = () => engine.setLoaded(false);
    const free = `${modelWords} ${t("cmd.model-unload.keys")}`;
    if (engine.unloaded) {
      out.push({ id: "model-load", group: "Aktionen", label: t("shell.load.loadAria"), hint: t("cmd.model-load.hint"), icon: "model", keywords: `${modelWords} ${t("cmd.model-load.keys")}`, kind: "modell-laden", run: () => engine.setLoaded(true) });
    } else {
      out.push({ id: "model-unload", group: "Aktionen", label: t("shell.load.unloadAria"), hint: t("cmd.model-unload.hint"), icon: "model", keywords: free, kind: "modell-entladen", run: unload });
      out.push({ id: "model-free-vram", group: "Aktionen", label: t("cmd.model-free-vram"), hint: t("cmd.model-free-vram.hint"), icon: "memory", keywords: free, kind: "modell-entladen", run: unload });
    }
  }
  out.push({
    id: "import",
    group: "Aktionen",
    label: t("cmd.import"),
    hint: t("cmd.import.hint"),
    icon: "upload",
    keywords: t("cmd.import.keys"),
    idle: 6,
    kind: "import",
    run: openImport,
  });
  out.push({
    id: "builder-new",
    group: "Aktionen",
    label: t("builder.cmd.new"),
    hint: t("builder.cmd.new.hint"),
    icon: "pencil",
    keywords: t("builder.cmd.new.keys"),
    kind: "figur-bauen",
    run: go("#/chars/new"),
  });
  out.push({
    id: "import-personas",
    group: "Aktionen",
    label: t("cmd.import-personas"),
    hint: t("cmd.import-personas.hint"),
    icon: "persona",
    keywords: `${personaWords} ${t("cmd.import-personas.keys")}`,
    kind: "import-personas",
    run: () => openSetting("#/import", "import-personas"),
  });
  out.push({
    id: "import-sillytavern",
    group: "Aktionen",
    label: t("cmd.import-sillytavern"),
    hint: t("cmd.import-sillytavern.hint"),
    icon: "folder",
    keywords: t("cmd.import-sillytavern.keys"),
    kind: "import-sillytavern",
    run: () => openSetting("#/import", "import-sillytavern"),
  });
  out.push({ id: "theme", group: "Aktionen", label: t("cmd.theme"), icon: "palette", kind: "farbschema-waehlen", fills: t("shell.general.look.themeNamed", { name: "" }) });
  out.push({ id: "layout", group: "Aktionen", label: t("cmd.layout"), icon: "layout", kind: "layout-waehlen", fills: t("shell.general.look.layoutNamed", { name: "" }) });
  if (personas.length > 1) {
    out.push({
      id: "persona",
      group: "Aktionen",
      label: t("cmd.persona"),
      hint: active ? t("cmd.persona.hint", { name: active.name }) : undefined,
      icon: "persona",
      keywords: `${personaWords} ${t("cmd.persona.keys")}`,
      kind: "persona-waehlen",
      fills: t("cmd.personaNamed", { name: "" }),
    });
  }
  out.push({
    id: "personas",
    group: "Aktionen",
    label: t("shell.persona.manage"),
    hint: t("cmd.personas.hint"),
    icon: "persona",
    keywords: `${personaWords} ${t("cmd.personas.keys")}`,
    kind: "personas-verwalten",
    run: go("#/personas"),
  });
  out.push({
    id: "persona-new",
    group: "Aktionen",
    label: t("shell.persona.new"),
    icon: "plus",
    keywords: `${personaWords} ${t("cmd.persona-new.keys")}`,
    kind: "persona-neu",
    run: go("#/personas/new"),
  });

  // Export lives in a character's chat overview: the one on screen, else the last one written with.
  const exportFrom =
    chars.find((c) => c.id === charId && c.chats > 0) ?? chars.find((c) => c.id === lead?.chat.character_id) ?? chars.find((c) => c.chats > 0);
  if (exportFrom) {
    out.push({
      id: "chat-export",
      group: "Aktionen",
      label: t("cmd.chat-export"),
      hint: exportFrom.name,
      icon: "download",
      keywords: t("cmd.chat-export.keys"),
      kind: "chat-exportieren",
      run: () => openSetting(`#/char/${exportFrom.id}/chats`, "chat-export"),
    });
  }

  // A character's picture, card export and "as persona": the one on screen, else the last one written with.
  const figure = chars.find((c) => c.id === charId) ?? chars.find((c) => c.id === lead?.chat.character_id) ?? chars[0];
  if (figure) {
    out.push({
      id: "char-picture",
      group: "Aktionen",
      label: t("cmd.char-picture"),
      hint: figure.name,
      where: t("cmd.where.figure"),
      icon: "upload",
      keywords: t("cmd.char-picture.keys"),
      boost: figure.id === charId ? 0.1 : 0,
      kind: "bild-setzen",
      run: () => openSetting(`#/char/${figure.id}/settings`, "char-picture"),
    });
    out.push({
      id: "card-export",
      group: "Aktionen",
      label: t("cmd.card-export"),
      hint: figure.name,
      icon: "download",
      keywords: t("cmd.card-export.keys"),
      boost: figure.id === charId ? 0.1 : 0,
      kind: "figur-exportieren",
      run: () => openSetting(`#/char/${figure.id}/settings`, "card-export"),
    });
    out.push({
      id: "as-persona",
      group: "Aktionen",
      label: t("cmd.as-persona"),
      hint: figure.name,
      icon: "persona",
      keywords: `${personaWords} ${t("cmd.as-persona.keys")}`,
      kind: "als-persona",
      run: () => openSetting(`#/char/${figure.id}/settings`, "as-persona"),
    });
  }
  if (active) {
    out.push({
      id: "persona-picture",
      group: "Aktionen",
      label: t("cmd.persona-picture"),
      hint: active.name,
      where: t("shell.nav.personas"),
      icon: "upload",
      keywords: `${personaWords} ${t("cmd.persona-picture.keys")}`,
      kind: "persona-bild",
      run: () => openSetting(`#/personas/${active.id}`, "persona-picture"),
    });
  }
  if (personas.length > 0) {
    out.push({
      id: "personas-export",
      group: "Aktionen",
      label: t("cmd.personas-export"),
      hint: t("cmd.personas-export.hint"),
      icon: "download",
      keywords: `${personaWords} ${t("cmd.personas-export.keys")}`,
      kind: "personas-exportieren",
      run: () => openSetting("#/personas", "personas-export"),
    });
  }

  if (chatId !== null && personas.length > 0) {
    out.push({
      id: "chat-persona",
      group: "Aktionen",
      label: t("cmd.chat-persona"),
      hint: t("cmd.chat-persona.hint"),
      icon: "pin",
      keywords: `${personaWords} ${t("cmd.chat-persona.keys")}`,
      boost: 0.1,
      kind: "chat-persona",
      run: () => window.dispatchEvent(new Event(CHAT_PERSONA_EVENT)),
    });
  }

  if (chatId !== null) {
    out.push({
      id: "chat-find",
      group: "Aktionen",
      label: t("cmd.chat-find"),
      hint: t("cmd.chat-find.hint"),
      icon: "search",
      keywords: t("cmd.chat-find.keys"),
      boost: 0.1,
      kind: "chat-suchen",
      run: () => window.dispatchEvent(new Event(CHAT_FIND_EVENT)),
    });
  }
  if (chars.length > 0) {
    out.push({
      id: "library-order",
      group: "Aktionen",
      label: t("cmd.library-order"),
      hint: t("cmd.library-order.hint"),
      icon: "updown",
      keywords: t("cmd.library-order.keys"),
      kind: "bibliothek-ordnen",
      run: () => openSetting("#/chars", "library-order"),
    });
    const libLayout = LIBRARY_LAYOUTS.find((l) => l.id === library?.library_layout) ?? LIBRARY_LAYOUTS[0];
    out.push({
      id: "library-layout",
      group: "Einstellungen",
      label: t("cmd.library-layout"),
      hint: libLayout.name,
      where: t("shell.nav.chars"),
      icon: "layout",
      keywords: t("cmd.library-layout.keys"),
      kind: "bibliothek-ansicht",
      run: () => openSetting("#/chars", "library-layout"),
    });
    for (const l of LIBRARY_LAYOUTS) {
      out.push({
        id: `library-layout-${l.id}`,
        group: "Aktionen",
        label: t("cmd.library-layout-as", { name: l.name }),
        hint: l.note,
        icon: "layout",
        current: l.id === libLayout.id,
        keywords: t("cmd.library-layout-as.keys"),
        kind: "bibliothek-ansicht",
        run: () => requestLibraryLayout(l.id),
      });
    }
  }
  const tagsOf = chars.find((c) => c.id === charId);
  if (tagsOf) {
    out.push({
      id: "own-tags",
      group: "Aktionen",
      label: t("cmd.own-tags"),
      hint: tagsOf.name,
      icon: "tag",
      keywords: t("cmd.own-tags.keys"),
      kind: "figur-tags",
      run: () => openSetting(`#/char/${tagsOf.id}/settings`, "own-tags"),
    });
  }
  if (figure) {
    out.push({
      id: "char-tagline",
      group: "Aktionen",
      label: t("cmd.char-tagline"),
      hint: figure.name,
      where: t("cmd.where.figure"),
      icon: "pencil",
      keywords: t("cmd.char-tagline.keys"),
      boost: figure.id === charId ? 0.1 : 0,
      kind: "figur-kurztext",
      run: () => openSetting(`#/char/${figure.id}/settings`, "char-tagline"),
    });
  }

  // Settings, each opening at its own place.
  out.push({
    id: "set-theme",
    group: "Einstellungen",
    label: t("shell.general.look.theme"),
    hint: themeName,
    where: t("shell.general.look.title"),
    icon: "palette",
    keywords: t("cmd.set-theme.keys"),
    boost: 0.05,
    idle: 7,
    kind: "einstellung",
    run: () => openSetting("#/settings", "theme"),
  });
  out.push({
    id: "set-name",
    group: "Einstellungen",
    label: t("cmd.set-name"),
    hint: library?.user_name || undefined,
    where: t("shell.nav.personas"),
    icon: "persona",
    keywords: t("cmd.set-name.keys"),
    boost: 0.05,
    idle: 8,
    kind: "einstellung",
    run: () => openSetting(active ? `#/personas/${active.id}` : "#/personas", "persona-name"),
  });
  // The fields of the active persona, each at its own place in the editor.
  const personaPlace = (spot: string) => () => openSetting(active ? `#/personas/${active.id}` : "#/personas", spot);
  const personaFields: [string, string, string, string][] = [
    ["pronouns", t("cmd.persona.pronouns"), t("cmd.persona.pronouns.keys"), active?.pronouns ?? ""],
    ["appearance", t("cmd.persona.appearance"), t("cmd.persona.appearance.keys"), ""],
    ["personality", t("cmd.persona.personality"), t("cmd.persona.personality.keys"), ""],
    ["background", t("cmd.persona.background"), t("cmd.persona.background.keys"), ""],
    ["description", t("cmd.persona.description"), t("cmd.persona.description.keys"), ""],
  ];
  for (const [key, label, keywords, hint] of personaFields) {
    out.push({
      id: `set-persona-${key}`,
      group: "Einstellungen",
      label,
      hint: hint || undefined,
      where: t("shell.nav.personas"),
      icon: "persona",
      keywords: `${keywords} ${t("cmd.persona.field.keys")}`,
      boost: 0.05,
      kind: "einstellung",
      run: personaPlace(`persona-${key}`),
    });
  }
  out.push({
    id: "set-persona-every-turn",
    group: "Einstellungen",
    label: t("cmd.set-persona-every-turn"),
    hint: active ? (active.every_turn ? t("cmd.set-persona-every-turn.every") : t("cmd.set-persona-every-turn.once")) : undefined,
    where: t("shell.nav.personas"),
    icon: "persona",
    keywords: t("cmd.set-persona-every-turn.keys"),
    boost: 0.05,
    kind: "einstellung",
    run: personaPlace("persona-every-turn"),
  });
  out.push({
    id: "set-persona-card-priority",
    group: "Einstellungen",
    label: t("cmd.set-persona-card-priority"),
    hint: active ? (active.card_priority === "card" ? t("cmd.set-persona-card-priority.card") : t("cmd.set-persona-card-priority.persona")) : undefined,
    where: t("shell.nav.personas"),
    icon: "persona",
    keywords: `${fitWords} ${t("cmd.set-persona-card-priority.keys")}`,
    boost: 0.05,
    kind: "einstellung",
    run: personaPlace("persona-card-priority"),
  });
  // The default persona of each character, and how its card fits you; the one on screen first.
  for (const c of [...chars].sort((a, b) => Number(b.id === charId) - Number(a.id === charId))) {
    out.push({
      id: `set-user-fit-${c.id}`,
      group: "Einstellungen",
      label: t("cmd.set-user-fit", { name: c.name }),
      hint: t(`cmd.fit.${c.user_check_status}`),
      where: t("cmd.where.figure"),
      icon: "persona",
      keywords: `${fitWords} ${t("cmd.set-user-fit.keys")}`,
      boost: c.id === charId ? 0.1 : 0,
      kind: "einstellung",
      run: () => openSetting(`#/char/${c.id}/settings`, "user-fit"),
    });
    const dflt = personas.find((p) => p.id === c.default_persona_id);
    out.push({
      id: `set-default-persona-${c.id}`,
      group: "Einstellungen",
      label: t("cmd.set-default-persona", { name: c.name }),
      hint: dflt ? dflt.name : t("cmd.none"),
      where: t("cmd.where.figure"),
      icon: "persona",
      keywords: t("cmd.set-default-persona.keys"),
      boost: c.id === charId ? 0.1 : 0,
      kind: "einstellung",
      run: () => openSetting(`#/char/${c.id}/settings`, "default-persona"),
    });
  }
  out.push({
    id: "set-layout",
    group: "Einstellungen",
    label: t("shell.general.look.layout"),
    hint: layoutName,
    where: t("shell.general.look.title"),
    icon: "layout",
    keywords: t("cmd.set-layout.keys"),
    boost: 0.05,
    kind: "einstellung",
    run: () => openSetting("#/settings", "layout"),
  });
  out.push({
    id: "set-language",
    group: "Einstellungen",
    label: t("shell.general.language.label"),
    hint: LANGS.find((l) => l.id === lang())?.name,
    where: t("shell.settings.tab.general"),
    icon: "sliders",
    // Every language's own name and the word "language" itself find it, whatever the app speaks now.
    keywords: `${t("cmd.set-language.keys")} language ${LANGS.map((l) => l.name).join(" ")}`,
    boost: 0.05,
    kind: "einstellung",
    run: () => openSetting("#/settings", "language"),
  });
  const hint = notifyPrefs();
  out.push({
    id: "set-answer-hint",
    group: "Einstellungen",
    label: t("shell.hint.done"),
    hint: hint.on ? (hint.sound ? t("cmd.set-answer-hint.sound") : t("cmd.set-answer-hint.silent")) : t("cmd.off"),
    where: t("shell.hint.title"),
    icon: "sliders",
    keywords: t("cmd.set-answer-hint.keys"),
    boost: 0.05,
    kind: "einstellung",
    run: () => openSetting("#/settings", "answer-hint"),
  });
  // The model, simple: what runs, switching it, the benchmark, unloading (tab "Modell").
  const modelPlace = (spot: string) => () => openSetting("#/settings/model", spot);
  const modelSettings: [string, string, string | undefined, string, string][] = [
    ["model", t("shell.nav.model"), model ?? undefined, "model", `${modelWords} ${t("cmd.model.model.keys")}`],
    ["switch-model", t("cmd.model.switch-model"), t("cmd.model.switch-model.hint"), "local-models", `${modelWords} ${t("cmd.model.switch-model.keys")}`],
    ["local-models", t("cmd.model.local-models"), t("cmd.model.local-models.hint"), "local-models", `${modelWords} ${t("cmd.model.local-models.keys")}`],
    ["benchmark", t("cmd.model.benchmark"), t("cmd.model.benchmark.hint"), "benchmark", t("cmd.model.benchmark.keys")],
    ["bench-measure", t("cmd.model.bench-measure"), t("cmd.model.bench-measure.hint"), "benchmark", t("cmd.model.bench-measure.keys")],
    ["idle-unload", t("cmd.model.idle-unload"), t("cmd.model.idle-unload.hint"), "idle-unload", `${modelWords} ${t("cmd.model.idle-unload.keys")}`],
    ["purpose-advice", t("cmd.model.purpose-advice"), t("cmd.model.purpose-advice.hint"), "benchmark", `${modelWords} ${t("cmd.model.purpose-advice.keys")}`],
  ];
  for (const [key, label, hint, spot, keywords] of modelSettings) {
    out.push({ id: `set-${key}`, group: "Einstellungen", label, hint, where: t("shell.settings.tab.model"), icon: "model", keywords, boost: 0.05, kind: "einstellung", run: modelPlace(spot) });
  }
  // How every chat reads (tab "Chat").
  const chatPlace = (spot: string) => () => openSetting("#/settings/chat", spot);
  const chatSettings: [string, string, string, string, string][] = [
    ["font", t("cmd.chat.font"), t("cmd.chat.font.hint"), "chat-font", t("cmd.chat.font.keys")],
    ["size", t("cmd.chat.size"), t("cmd.chat.size.hint"), "chat-size", t("cmd.chat.size.keys")],
    ["line", t("cmd.chat.line"), t("cmd.chat.line.hint"), "chat-line", t("cmd.chat.line.keys")],
    ["width", t("cmd.chat.width"), t("cmd.chat.width.hint"), "chat-width", t("cmd.chat.width.keys")],
    ["style", t("cmd.chat.style"), t("cmd.chat.style.hint"), "chat-style", t("cmd.chat.style.keys")],
    ["speech", t("cmd.chat.speech"), t("cmd.chat.speech.hint"), "chat-speech", t("cmd.chat.speech.keys")],
    ["animate", t("cmd.chat.animate"), t("cmd.chat.animate.hint"), "chat-animate", t("cmd.chat.animate.keys")],
    ["timings", t("cmd.chat.timings"), t("cmd.chat.timings.hint"), "chat-timings", t("cmd.chat.timings.keys")],
    ["say", t("cmd.chat.say"), t("cmd.chat.say.hint"), "chat-say", t("cmd.chat.say.keys")],
    ["act", t("cmd.chat.act"), t("cmd.chat.act.hint"), "chat-act", t("cmd.chat.act.keys")],
    ["act_italic", t("cmd.chat.act_italic"), t("cmd.chat.act_italic.hint"), "chat-act_italic", t("cmd.chat.act_italic.keys")],
    ["say_bold", t("cmd.chat.say_bold"), t("cmd.chat.say_bold.hint"), "chat-say_bold", t("cmd.chat.say_bold.keys")],
    ["mine", t("cmd.chat.mine"), t("cmd.chat.mine.hint"), "chat-mine", t("cmd.chat.mine.keys")],
    ["avatars", t("cmd.chat.avatars"), t("cmd.chat.avatars.hint"), "chat-avatars", t("cmd.chat.avatars.keys")],
    ["para", t("cmd.chat.para"), t("cmd.chat.para.hint"), "chat-para", t("cmd.chat.para.keys")],
    ["sides", t("cmd.chat.sides"), t("cmd.chat.sides.hint"), "chat-style", t("cmd.chat.sides.keys")],
    // Layout, bubbles, name/time/picture and new messages (texts in the chat dictionary).
    ["wide", t("chat.cmd.wide"), t("chat.cmd.wide.hint"), "chat-style", t("chat.cmd.wide.keys")],
    ["flip", t("chat.cmd.flip"), t("chat.cmd.flip.hint"), "chat-style", t("chat.cmd.flip.keys")],
    ["edge", t("chat.cmd.edge"), t("chat.cmd.edge.hint"), "chat-style", t("chat.cmd.edge.keys")],
    ["gap", t("chat.cmd.gap"), t("chat.cmd.gap.hint"), "chat-gap", t("chat.cmd.gap.keys")],
    ["bubble", t("chat.cmd.bubble"), t("chat.cmd.bubble.hint"), "chat-bubble", t("chat.cmd.bubble.keys")],
    ["shape", t("chat.cmd.shape"), t("chat.cmd.shape.hint"), "chat-corner", t("chat.cmd.shape.keys")],
    ["fills", t("chat.cmd.fills"), t("chat.cmd.fills.hint"), "chat-ai_fill", t("chat.cmd.fills.keys")],
    ["glass", t("chat.cmd.glass"), t("chat.cmd.glass.hint"), "chat-fill", t("chat.cmd.glass.keys")],
    ["shadow", t("chat.cmd.shadow"), t("chat.cmd.shadow.hint"), "chat-shadow", t("chat.cmd.shadow.keys")],
    ["story", t("chat.cmd.story"), t("chat.cmd.story.hint"), "chat-story", t("chat.cmd.story.keys")],
    ["think", t("chat.cmd.think"), t("chat.cmd.think.hint"), "chat-think", t("chat.cmd.think.keys")],
    ["names", t("chat.cmd.names"), t("chat.cmd.names.hint"), "chat-names", t("chat.cmd.names.keys")],
    ["times", t("chat.cmd.times"), t("chat.cmd.times.hint"), "chat-times", t("chat.cmd.times.keys")],
    ["avatar-look", t("chat.cmd.avatar"), t("chat.cmd.avatar.hint"), "chat-avatars", t("chat.cmd.avatar.keys")],
    ["enter", t("chat.cmd.enter"), t("chat.cmd.enter.hint"), "chat-enter", t("chat.cmd.enter.keys")],
    ["typing", t("chat.cmd.typing"), t("chat.cmd.typing.hint"), "chat-typing", t("chat.cmd.typing.keys")],
  ];
  for (const [key, label, hint, spot, keywords] of chatSettings) {
    out.push({ id: `set-chat-${key}`, group: "Einstellungen", label, hint, where: t("shell.settings.tab.chat"), icon: "chat", keywords, boost: 0.05, kind: "einstellung", run: chatPlace(spot) });
  }
  // Every model setting exactly (tab "Experte"): profiles, the sampler, the start options, engines, folders, the PC.
  const expertPlace = (spot: string) => () => openSetting("#/settings/expert", spot);
  const expertSettings: [string, string, string, string, string][] = [
    ["expert", t("shell.settings.tab.expert"), t("cmd.expert.expert.hint"), "profiles", `${modelWords} ${t("cmd.expert.expert.keys")}`],
    ["profile", t("cmd.expert.profile"), t("cmd.expert.profile.hint"), "profiles", `${modelWords} ${t("cmd.expert.profile.keys")}`],
    ["temperature", t("cmd.expert.temperature"), t("cmd.expert.temperature.hint"), "temperature", t("cmd.expert.temperature.keys")],
    ["creativity", t("cmd.expert.creativity"), t("cmd.expert.creativity.hint"), "temperature", t("cmd.expert.creativity.keys")],
    ["n-predict", t("cmd.expert.n-predict"), t("cmd.expert.n-predict.hint"), "n-predict", t("cmd.expert.n-predict.keys")],
    ["sampler", t("cmd.expert.sampler"), t("cmd.expert.sampler.hint"), "sampler", t("cmd.expert.sampler.keys")],
    ["min-p", t("cmd.expert.min-p"), t("cmd.expert.pick.hint"), "min-p", t("cmd.expert.min-p.keys")],
    ["top-p", t("cmd.expert.top-p"), t("cmd.expert.pick.hint"), "top-p", t("cmd.expert.top-p.keys")],
    ["top-k", t("cmd.expert.top-k"), t("cmd.expert.pick.hint"), "top-k", t("cmd.expert.top-k.keys")],
    ["repeat-penalty", t("cmd.expert.repeat-penalty"), t("cmd.expert.repeat-penalty.hint"), "repeat-penalty", t("cmd.expert.repeat-penalty.keys")],
    ["dry", t("cmd.expert.dry"), t("cmd.expert.dry.hint"), "dry", t("cmd.expert.dry.keys")],
    ["xtc", t("cmd.expert.xtc"), t("cmd.expert.xtc.hint"), "xtc", t("cmd.expert.xtc.keys")],
    ["seed", t("cmd.expert.seed"), t("cmd.expert.seed.hint"), "seed", t("cmd.expert.seed.keys")],
    ["reasoning", t("cmd.expert.reasoning"), t("cmd.expert.reasoning.hint"), "reasoning", t("cmd.expert.reasoning.keys")],
    ["context", t("cmd.expert.context"), t("cmd.expert.context.hint"), "context", t("cmd.expert.context.keys")],
    ["gpu-layers", t("cmd.expert.gpu-layers"), t("cmd.expert.gpu-layers.hint"), "gpu-layers", t("cmd.expert.gpu-layers.keys")],
    ["n-cpu-moe", t("cmd.expert.n-cpu-moe"), t("cmd.expert.n-cpu-moe.hint"), "n-cpu-moe", t("cmd.expert.n-cpu-moe.keys")],
    ["kv-cache", t("cmd.expert.kv-cache"), t("cmd.expert.kv-cache.hint"), "kv-cache", t("cmd.expert.kv-cache.keys")],
    ["flash-attn", t("cmd.expert.flash-attn"), t("cmd.expert.flash-attn.hint"), "flash-attn", t("cmd.expert.flash-attn.keys")],
    ["threads", t("cmd.expert.threads"), t("cmd.expert.threads.hint"), "threads", t("cmd.expert.threads.keys")],
    ["batch", t("cmd.expert.batch"), t("cmd.expert.batch.hint"), "batch", t("cmd.expert.batch.keys")],
    ["mmap", t("cmd.expert.mmap"), t("cmd.expert.mmap.hint"), "mmap", t("cmd.expert.mmap.keys")],
    ["extra-args", t("cmd.expert.extra-args"), t("cmd.expert.extra-args.hint"), "extra-args", t("cmd.expert.extra-args.keys")],
    ["pc", t("cmd.expert.pc"), t("cmd.expert.pc.hint"), "pc", t("cmd.expert.pc.keys")],
    ["gpu", t("cmd.expert.gpu"), t("cmd.expert.gpu.hint"), "pc", t("cmd.expert.gpu.keys")],
    ["driver", t("cmd.expert.driver"), t("cmd.expert.driver.hint"), "pc", t("cmd.expert.driver.keys")],
    ["engine", t("cmd.expert.engine"), t("cmd.expert.engine.hint"), "engines", t("cmd.expert.engine.keys")],
    ["model-folders", t("cmd.expert.model-folders"), t("cmd.expert.model-folders.hint"), "model-folders", t("cmd.expert.model-folders.keys")],
    ["engine-port", t("cmd.expert.engine-port"), t("cmd.expert.engine-port.hint"), "engine-port", `${modelWords} ${t("cmd.expert.engine-port.keys")}`],
  ];
  for (const [key, label, hint, spot, keywords] of expertSettings) {
    out.push({ id: `set-${key}`, group: "Einstellungen", label, hint, where: t("shell.settings.tab.expert"), icon: "sliders", keywords, boost: 0.05, kind: "einstellung", run: expertPlace(spot) });
  }
  // Finding and downloading models: the tab "Vorschläge".
  const findPlace = (spot: string) => () => openSetting("#/settings/suggestions", spot);
  const findSettings: [string, string, string, string, string][] = [
    ["suggestions", t("shell.nav.modelsFind"), t("cmd.find.suggestions.hint"), "suggestions", `${modelWords} ${t("cmd.find.suggestions.keys")}`],
    ["best-model", t("cmd.find.best-model"), t("cmd.find.best-model.hint"), "best-models", `${modelWords} ${t("cmd.find.best-model.keys")}`],
    ["best-uncensored", t("cmd.find.best-uncensored"), t("cmd.find.best-uncensored.hint"), "best-uncensored", `${modelWords} ${t("cmd.find.best-uncensored.keys")}`],
    ["model-filter", t("cmd.find.model-filter"), t("cmd.find.model-filter.hint"), "model-filter", `${modelWords} ${t("cmd.find.model-filter.keys")}`],
    ["downloads", t("cmd.find.downloads"), t("cmd.find.downloads.hint"), "downloads", `${modelWords} ${t("cmd.find.downloads.keys")}`],
    ["hf-link", t("cmd.find.hf-link"), t("cmd.find.hf-link.hint"), "hf-link", `${modelWords} ${t("cmd.find.hf-link.keys")}`],
    ["find-more", t("cmd.find.find-more"), t("cmd.find.find-more.hint"), "find-more", `${modelWords} ${t("cmd.find.find-more.keys")}`],
    ["model-purpose", t("cmd.find.model-purpose"), t("cmd.find.model-purpose.hint"), "model-purpose", `${modelWords} ${t("cmd.find.model-purpose.keys")}`],
  ];
  for (const [key, label, hint, spot, keywords] of findSettings) {
    out.push({ id: `set-${key}`, group: "Einstellungen", label, hint, where: t("shell.settings.tab.suggestions"), icon: key === "suggestions" ? "model" : "download", keywords, boost: 0.05, kind: "einstellung", run: findPlace(spot) });
  }
  out.push({
    id: "set-logs",
    group: "Einstellungen",
    label: t("shell.settings.tab.logs"),
    hint: t("cmd.set-logs.hint"),
    where: t("shell.settings.tab.logs"),
    icon: "list",
    keywords: t("cmd.set-logs.keys"),
    boost: 0.05,
    kind: "einstellung",
    run: () => openSetting("#/settings/logs", "logs"),
  });
  out.push({
    id: "set-export",
    group: "Einstellungen",
    label: t("cmd.set-export"),
    hint: t("cmd.set-export.hint"),
    where: t("shell.settings.tab.logs"),
    icon: "download",
    keywords: t("cmd.set-export.keys"),
    boost: 0.05,
    kind: "einstellung",
    run: () => openSetting("#/settings/logs", "export"),
  });

  out.push({
    id: "set-backups",
    group: "Einstellungen",
    label: t("shell.settings.tab.backups"),
    hint: t("cmd.set-backups.hint"),
    where: t("shell.settings.tab.backups"),
    icon: "memory",
    keywords: t("cmd.set-backups.keys"),
    boost: 0.05,
    kind: "einstellung",
    run: () => openSetting("#/settings/backups", "backups"),
  });
  out.push({
    id: "set-backup-now",
    group: "Einstellungen",
    label: t("cmd.set-backup-now"),
    hint: t("cmd.set-backup-now.hint"),
    where: t("shell.settings.tab.backups"),
    icon: "download",
    keywords: t("cmd.set-backup-now.keys"),
    boost: 0.04,
    kind: "einstellung",
    run: () => openSetting("#/settings/backups", "backup-now"),
  });
  out.push({
    id: "set-backup-restore",
    group: "Einstellungen",
    label: t("cmd.set-backup-restore"),
    hint: t("cmd.set-backup-restore.hint"),
    where: t("shell.settings.tab.backups"),
    icon: "rewind",
    keywords: t("cmd.set-backup-restore.keys"),
    boost: 0.04,
    kind: "einstellung",
    run: () => openSetting("#/settings/backups", "backups"),
  });
  out.push({
    id: "set-backup-delete",
    group: "Einstellungen",
    label: t("cmd.set-backup-delete"),
    hint: t("cmd.set-backup-delete.hint"),
    where: t("shell.settings.tab.backups"),
    icon: "trash",
    keywords: t("cmd.set-backup-delete.keys"),
    boost: 0.03,
    kind: "einstellung",
    run: () => openSetting("#/settings/backups", "backups"),
  });
  out.push({
    id: "set-backup-limit",
    group: "Einstellungen",
    label: t("cmd.set-backup-limit"),
    hint: t("cmd.set-backup-limit.hint"),
    where: t("shell.settings.tab.backups"),
    icon: "memory",
    keywords: t("cmd.set-backup-limit.keys"),
    boost: 0.03,
    kind: "einstellung",
    run: () => openSetting("#/settings/backups", "backup-limit"),
  });
  out.push({
    id: "set-backup-older",
    group: "Einstellungen",
    label: t("cmd.set-backup-older"),
    hint: t("cmd.set-backup-older.hint"),
    where: t("shell.settings.tab.backups"),
    icon: "trash",
    keywords: t("cmd.set-backup-older.keys"),
    boost: 0.03,
    kind: "einstellung",
    run: () => openSetting("#/settings/backups", "backup-cleanup"),
  });
  out.push({
    id: "set-backup-many",
    group: "Einstellungen",
    label: t("cmd.set-backup-many"),
    hint: t("cmd.set-backup-many.hint"),
    where: t("shell.settings.tab.backups"),
    icon: "trash",
    keywords: t("cmd.set-backup-many.keys"),
    boost: 0.02,
    kind: "einstellung",
    run: () => openSetting("#/settings/backups", "backup-pick"),
  });

  // Phones in the Wi-Fi: on the PC the switch and the QR code; on a phone the connection.
  if (!onOtherDevice) {
    out.push({
      id: "set-phone",
      group: "Einstellungen",
      label: t("shell.nav.phone"),
      hint: t("cmd.set-phone.hint"),
      where: t("shell.settings.tab.phone"),
      icon: "qr",
      keywords: t("cmd.set-phone.keys"),
      boost: 0.05,
      kind: "einstellung",
      run: () => openSetting("#/settings/phone", "phone"),
    });
    out.push({
      id: "set-phone-cut",
      group: "Einstellungen",
      label: t("cmd.set-phone-cut"),
      hint: t("cmd.set-phone-cut.hint"),
      where: t("shell.settings.tab.phone"),
      icon: "close",
      keywords: t("cmd.set-phone-cut.keys"),
      boost: 0.03,
      kind: "einstellung",
      run: () => openSetting("#/settings/phone", "phone-cut"),
    });
  } else {
    out.push({
      id: "set-other-pc",
      group: "Einstellungen",
      label: inApp ? t("shell.nav.otherPc") : t("shell.nav.connection"),
      hint: inApp ? t("cmd.set-other-pc.hintApp") : t("cmd.set-other-pc.hintWeb"),
      where: t("shell.settings.tab.connection"),
      icon: "qr",
      keywords: t("cmd.set-other-pc.keys"),
      boost: 0.04,
      kind: "einstellung",
      run: inApp ? () => location.assign(RESCAN_URL) : () => openSetting("#/settings/phone", "phone"),
    });
  }

  recent.forEach((r, i) =>
    out.push({
      id: `chat-${r.chat.id}`,
      group: "Chats",
      label: r.chat.title ? `${r.name} · ${r.chat.title}` : r.name,
      hint: snippet(r.chat.last) || when(r.chat.updated_at),
      who: { ...r, id: r.chat.character_id },
      loose: t("cmd.chat.loose"),
      idle: i > 0 && i < 4 ? 1 + i : undefined,
      kind: "chat-oeffnen",
      run: go(`#/chat/${r.chat.id}`),
    }),
  );
  for (const c of chars) {
    out.push({ id: `char-${c.id}`, group: "Figuren", label: c.name, hint: t("cmd.char.hint"), who: c, loose: t("cmd.char.loose"), kind: "figur-oeffnen", run: go(`#/char/${c.id}/chats`) });
    out.push({ id: `new-${c.id}`, group: "Figuren", label: t("cmd.newWith", { name: c.name }), icon: "plus", kind: "neuer-chat", run: () => newChat(c.id) });
  }

  out.push({ id: "p-start", group: "Seiten", label: t("shell.nav.start"), icon: "home", keywords: t("cmd.p-start.keys"), kind: "seite", run: go("#/") });
  out.push({ id: "p-chars", group: "Seiten", label: t("shell.nav.chars"), icon: "users", keywords: t("cmd.p-chars.keys"), kind: "seite", run: go("#/chars") });
  out.push({ id: "p-settings", group: "Seiten", label: t("shell.nav.settings"), icon: "gear", keywords: t("cmd.p-settings.keys"), kind: "seite", run: go("#/settings") });

  for (const th of THEMES) {
    out.push({
      id: `theme-${th.id}`,
      group: "Farbschema",
      label: t("shell.general.look.themeNamed", { name: th.name }),
      hint: th.id === "papier" ? t("shell.theme.light") : t("shell.theme.dark"),
      swatch: th.id,
      current: theme === th.id,
      keywords: t(`cmd.words.theme.${th.id}`),
      kind: "farbschema",
      run: () => setTheme(th.id),
    });
  }
  for (const p of personas) {
    out.push({
      id: `persona-${p.id}`,
      group: "Persona",
      label: t("cmd.personaNamed", { name: p.name }),
      hint: p.id === activeId ? t("cmd.active") : undefined,
      who: p,
      whoKind: "persona",
      current: p.id === activeId,
      kind: "persona",
      run: () => setPersona(p.id),
    });
  }
  for (const l of HOME_LAYOUTS) {
    out.push({ id: `layout-${l.id}`, group: "Start-Layout", label: t("shell.general.look.layoutNamed", { name: l.name }), hint: l.note, icon: "layout", current: layout === l.id, kind: "layout", run: () => setLayout(l.id) });
  }
  // How the app behaves (tab "Allgemein", general/Rows.tsx): its start, sending, asking first, its data.
  const g = general();
  const onOff = (on: boolean) => (on ? t("cmd.on") : t("cmd.off"));
  const generalPlace = (spot: string) => () => openSetting("#/settings", spot);
  const generalSettings: [key: string, label: string, hint: string, where: string, spot: string, keywords: string][] = [
    ["start-page", t("shell.start.open"), t(`shell.start.page.${g.start_page}`), t("shell.start.title"), "start-page", t("cmd.general.start-page.keys")],
    ["autostart", t("shell.start.autostart"), g.autostart ? onOff(g.autostart.on) : t("cmd.desktopOnly"), t("shell.start.title"), "autostart", t("cmd.general.autostart.keys")],
    ["start-minimized", t("cmd.general.start-minimized"), g.autostart ? onOff(g.autostart.minimized) : t("cmd.desktopOnly"), t("shell.start.title"), g.autostart ? "start-minimized" : "autostart", t("cmd.general.start-minimized.keys")],
    ["send-key", t("cmd.general.send-key"), g.send_key === "enter" ? "Enter" : t("cmd.general.send-key.ctrl"), t("shell.write.title"), "send-key", t("cmd.general.send-key.keys")],
    ["confirm-delete", t("shell.ask.delete"), onOff(g.confirm_delete), t("shell.ask.title"), "confirm-delete", t("cmd.general.confirm-delete.keys")],
    ["confirm-rewind", t("shell.ask.rewind"), onOff(g.confirm_rewind), t("shell.ask.title"), "confirm-rewind", t("cmd.general.confirm-rewind.keys")],
    ["undo-time", t("cmd.general.undo-time"), t("shell.ask.seconds", { n: g.undo_secs }), t("shell.ask.title"), "undo-time", t("cmd.general.undo-time.keys")],
    ["backup-rhythm", t("shell.data.auto"), g.backup_rhythm === "off" ? t("cmd.off") : t(`cmd.general.backup-rhythm.${g.backup_rhythm}`), t("shell.data.title"), "backup-rhythm", t("cmd.general.backup-rhythm.keys")],
    ["data-folder", t("cmd.general.data-folder"), t("cmd.general.data-folder.hint"), t("shell.data.title"), "data-folder", t("cmd.general.data-folder.keys")],
  ];
  for (const [key, label, hint, where, spot, keywords] of generalSettings) {
    out.push({ id: `set-general-${key}`, group: "Einstellungen", label, hint, where, icon: "sliders", keywords, boost: 0.05, kind: "einstellung", run: generalPlace(spot) });
  }
  // The welcome tour of the first start, from its beginning (src/tutorial/).
  out.push({
    id: "tutorial-restart",
    group: "Einstellungen",
    label: t("tutorial.cmd.label"),
    where: t("shell.start.title"),
    icon: "refresh",
    keywords: t("tutorial.cmd.keys"),
    kind: "einfuehrung",
    run: () => void restartTour(),
  });
  return out;
}
