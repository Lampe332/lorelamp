// Finds German that was left behind in a screen shown in another language (leftovers.ts).
// Fixed rules, no dictionary lookup: letters only German has, and words that are common in the
// app's German texts and are no words of English, Spanish or Russian.

export type Lang = "en" | "es" | "ru";

/** Words of the app's German texts. Lower case; none of them is an English or Spanish word
 *  ("die", "was", "an", "in", "Start" and "Chat" are, so they are not here). */
const WORDS = (
  "der das den dem des und oder nicht mit für fuer von vom zum zur ist sind eine einen einem einer wird werden auch noch nur beim nach " +
  "über unter wenn dann aber dein deine deinen du ich wie wo wer hier jetzt immer kein keine " +
  "einstellungen einstellung suchen suche charaktere figur figuren karte karten modell modelle sicherung sicherungen handy vorschläge " +
  "experte allgemein mehr zuletzt spielen welt laden entladen farbschema darstellung sprache hinweise hinweis schreiben nachfragen daten " +
  "verbinden verbindung wechseln öffnen schließen löschen speichern bearbeiten rückgängig abbrechen auswählen treffer ergebnisse bald " +
  "startseite nachricht nachrichten antwort gerät geräte ordner fehler neuer neue neues alle heute gestern morgen täglich wöchentlich aus"
).split(" ");
// "aus" is German for "off" and no word of the other three; "morgen" and "alle" likewise.

/** Names and terms that stay as they are in every language. Add proper names here, nothing else. */
export const KEEP = ["Lorelamp", "SillyTavern", "RisuAI", "Hugging Face", "Messenger", "Bordeaux", "Bento", "Dashboard", "Explorer", "Windows"];

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** What reads as German in `text` shown in `lang` (empty: nothing). `keep`: further names to leave
 *  out, e.g. the test data's character and persona names. */
export function germanLeft(text: string, lang: Lang, keep: string[] = []): string[] {
  let rest = text;
  for (const name of [...KEEP, ...keep].filter(Boolean).sort((a, b) => b.length - a.length)) rest = rest.replace(new RegExp(escape(name), "gi"), " ");
  const found = new Set<string>();
  // ä, ö and ß are in none of the other languages; ü is in a few Spanish words ("pingüino").
  const letters = lang === "es" ? /[äöÄÖß]/ : /[äöüÄÖÜß]/;
  for (const word of rest.split(/[^\p{L}]+/u)) {
    if (!word) continue;
    if (letters.test(word) || WORDS.includes(word.toLowerCase())) found.add(word);
  }
  return [...found];
}
