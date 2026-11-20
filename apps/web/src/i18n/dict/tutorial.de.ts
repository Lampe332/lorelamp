// The welcome tour of a new installation (src/tutorial/), its row in Settings → Allgemein and its
// quick action. German is the source of the keys.

import type { Source } from "../types";

export const tutorialDe = {
  "tutorial.dialog": "Einführung in Lorelamp",
  "tutorial.later": "Später",
  "tutorial.back": "Zurück",
  "tutorial.skip": "Überspringen",
  "tutorial.next": "Weiter",
  "tutorial.finish": "Fertig",
  "tutorial.step": "Schritt {step} von {all}",
  "tutorial.steps": "Fortschritt",
  "tutorial.failed": "Das hat nicht geklappt ({error}).",

  // ---- welcome: language, what the app is, who made it
  "tutorial.welcome.lead": "Hier erzählst du Geschichten mit Figuren, die dir antworten. Das Modell dahinter läuft auf deinem eigenen PC.",
  "tutorial.welcome.language": "Sprache",
  "tutorial.welcome.start": "Los geht's",
  "tutorial.welcome.by": "Gemacht von {name}",
  "tutorial.welcome.code": "Quellcode auf GitHub",
  "tutorial.welcome.again": "Die Einführung kannst du jederzeit unter Einstellungen, Allgemein neu starten.",

  // ---- 1: the model
  "tutorial.model.title": "Ein Modell einrichten",
  "tutorial.model.lead": "Das Modell schreibt, was deine Figuren sagen. Es läuft bei dir, ohne Konto und ohne Abo.",
  "tutorial.model.ready": "Eingerichtet: {name}",
  "tutorial.model.readySome": "Ein Modell ist eingerichtet.",
  "tutorial.model.todo": "Lorelamp schlägt dir Modelle vor, die auf deinen PC passen. Ein Download dauert je nach Größe ein paar Minuten.",
  "tutorial.model.atPc": "Das Modell richtest du am PC ein. Hier kannst du einfach weitermachen.",

  // ---- 2: a character
  "tutorial.figure.title": "Eine Figur holen",
  "tutorial.figure.lead": "Figuren kommen als Karte, als PNG oder JSON, etwa aus SillyTavern oder von Chub. Oder du baust dir selbst eine.",
  "tutorial.figure.there": { one: "Du hast schon {n} Figur.", other: "Du hast schon {n} Figuren." },
  "tutorial.figure.import": "Karte importieren",
  "tutorial.figure.build": "Neue Figur bauen",

  // ---- 3: who the user is
  "tutorial.persona.title": "Wer bist du in der Geschichte?",
  "tutorial.persona.lead": "Mit diesem Namen sprechen dich die Figuren an. Aussehen, Art und Vorgeschichte kannst du später in einer Persona ergänzen.",
  "tutorial.persona.name": "Dein Name",
  "tutorial.persona.placeholder": "zum Beispiel Alex",
  "tutorial.persona.saved": "Gespeichert.",
  "tutorial.persona.more": "Persona ausführlich anlegen",

  // ---- 4: the first chat
  "tutorial.chat.title": "Dein erster Chat",
  "tutorial.chat.lead": "Such dir jemanden aus. Die Figur fängt an, du antwortest unten im Feld.",
  "tutorial.chat.none": "Sobald du eine Figur hast, startest du hier mit einem Tipp deinen ersten Chat.",
  "tutorial.chat.toFigure": "Figur holen",
  "tutorial.chat.with": "Chat mit {name} beginnen",
  "tutorial.chat.quiet": "Solange kein Modell läuft, bleibt die Antwort aus. Öffnen kannst du den Chat trotzdem.",

  // ---- while the user does a step elsewhere in the app
  "tutorial.dock": "Weiter mit der Einführung",

  // ---- Settings → Allgemein, and the quick action
  "tutorial.settings.label": "Einführung",
  "tutorial.settings.note": "Die kurze Tour vom ersten Start: Sprache, Modell, Figur, dein Name, erster Chat.",
  "tutorial.settings.restart": "Neu starten",
  "tutorial.cmd.label": "Einführung neu starten",
  "tutorial.cmd.keys": "tutorial tour einführung anleitung hilfe erste schritte willkommen onboarding",
} as const satisfies Source;
