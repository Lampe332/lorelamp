//! Prompt building, layout B from `docs/cache.md`:
//! - HEAD (system message): never changes during a chat, so it is read once and then cached.
//! - HISTORY: only grows at the end; once it outgrows the model's context, its oldest turns after
//!   the first message are left out in sticky blocks ([`cut`]).
//! - TAIL: everything that applies to the next reply only, appended to the last user message.

use serde::{Deserialize, Serialize};

use crate::card::Card;

/// Narration rules, written in German for every language of the app; `{{language_rule}}` is the one
/// line that says which language the replies are in ([`Language::rule`]).
const BASE: &str = "\
Du schreibst ein fiktives Rollenspiel. Du spielst {{char}} sowie alle Nebenfiguren und erzählst die Welt. Der Nutzer spielt {{user}}.

Regeln:
- Bleib in der Figur. Schreibe nie Handlungen, Gedanken oder wörtliche Rede für {{user}}.
- Zeigen statt erzählen: Handlung, Sinneseindrücke und wörtliche Rede statt Zusammenfassung.
- Treibe die Szene voran und wiederhole dich nicht. Kein Kommentar außerhalb der Geschichte.
- Wörtliche Rede in Anführungszeichen, Gedanken *kursiv*, Erzählung als Fließtext.
- {{language_rule}}
";

/// Templates of the Gemma family require strictly alternating turns starting with the user, so a
/// chat that opens with the character's greeting gets this fixed opener in front (static = cached).
const OPENER: &str = "(Die Szene beginnt.)";

/// The language the figures answer in: the app's language (setting `language`). German is the
/// default and leaves the prompt as it always was (replies follow the user's language). Any other
/// language is asked for twice, as layout B wants it: one fixed line in the static head and a short
/// note in the per-reply content at the end of the last user message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    De,
    En,
    Es,
    Ru,
}

impl Language {
    pub const ALL: [Language; 4] = [Language::De, Language::En, Language::Es, Language::Ru];

    pub fn parse(s: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.as_str() == s)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Language::De => "de",
            Language::En => "en",
            Language::Es => "es",
            Language::Ru => "ru",
        }
    }

    /// The language line of the head's rules: in German like the rules around it, then once more
    /// in the language itself.
    fn rule(self) -> &'static str {
        match self {
            Language::De => "Schreibe in der Sprache, in der der Nutzer schreibt (Standard: Deutsch).",
            Language::En => "Schreibe deine Antworten immer auf Englisch, auch wenn die Figurenkarte, diese Regeln oder der Nutzer eine andere Sprache verwenden. Always write your replies in English.",
            Language::Es => "Schreibe deine Antworten immer auf Spanisch, auch wenn die Figurenkarte, diese Regeln oder der Nutzer eine andere Sprache verwenden. Escribe siempre tus respuestas en español.",
            Language::Ru => "Schreibe deine Antworten immer auf Russisch, auch wenn die Figurenkarte, diese Regeln oder der Nutzer eine andere Sprache verwenden. Всегда пиши свои ответы на русском языке.",
        }
    }

    /// The per-reply reminder; none for German.
    fn note(self) -> Option<&'static str> {
        match self {
            Language::De => None,
            Language::En => Some("Reply in English."),
            Language::Es => Some("Responde en español."),
            Language::Ru => Some("Отвечай на русском языке."),
        }
    }

    /// Words one token of a reply makes on average (German and Russian words take more tokens).
    fn words_per_token(self) -> f64 {
        match self {
            Language::De => 0.6,
            Language::En => 0.7,
            Language::Es => 0.65,
            Language::Ru => 0.55,
        }
    }

    /// The length note in this language: about `words`, at most `most` words, ending cleanly.
    fn length_line(self, words: u32, most: u32) -> String {
        match self {
            Language::De => format!("Länge: etwa {words} Wörter, höchstens {most}. Hör mit einem vollständigen Satz auf, nicht mitten im Gedanken."),
            Language::En => format!("Length: about {words} words, never more than {most}. Finish with a complete sentence; do not stop mid-thought."),
            Language::Es => format!("Extensión: unas {words} palabras, nunca más de {most}. Termina con una frase completa; no te detengas a mitad de una idea."),
            Language::Ru => format!("Объём: около {words} слов, не больше {most}. Закончи полным предложением, не обрывай мысль на середине."),
        }
    }
}

/// The per-reply note on how long the reply should be, for an answer length of `tokens` tokens
/// (`Sampler::reply_length`): about the words of [`crate::length::MARK_PERCENT`] of it (where the
/// reply may end), at most the words of the length itself (the hard cap, which a reply never
/// passes). None above [`crate::length::NOTE_MAX_TOKENS`]: such a length is only an upper bound.
pub fn length_note(lang: Language, tokens: i64) -> Option<String> {
    if tokens > crate::length::NOTE_MAX_TOKENS {
        return None;
    }
    let most_tokens = tokens.max(1);
    let tokens = (most_tokens * crate::length::MARK_PERCENT as i64 / 100).max(1);
    let words = |t: i64| {
        let w = t as f64 * lang.words_per_token();
        let step = if w < 100.0 { 5.0 } else { 10.0 };
        ((w / step).round() * step).max(5.0) as u32
    };
    let (about, most) = (words(tokens), words(most_tokens));
    Some(lang.length_line(about, most.max(about + 5)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub role: Role,
    pub content: String,
}

/// One message for llama-server's `/apply-template`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatMsg {
    pub role: &'static str,
    pub content: String,
}

/// Resolves the macros the app knows; unknown `{{...}}` stay untouched.
pub fn resolve(text: &str, char_name: &str, user_name: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            rest = &rest[open..];
            break;
        };
        match after[..close].trim().to_lowercase().as_str() {
            "char" => out.push_str(char_name),
            "user" => out.push_str(user_name),
            "original" => {} // the app's own rules always stay in place, nothing to splice in
            _ => out.push_str(&rest[open..open + 2 + close + 2]),
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out.replace("<USER>", user_name).replace("<BOT>", char_name)
}

/// The character's opening messages for a new chat: `first_mes`, then every alternate greeting,
/// macros resolved, empty ones left out. The first is the message's text, all of them are its
/// variants (`messages.swipes`). Empty: the chat starts without a greeting.
pub fn greetings(card: &Card, user_name: &str) -> Vec<String> {
    std::iter::once(&card.first_mes)
        .chain(&card.alternate_greetings)
        .map(|g| resolve(g, &card.name, user_name))
        .filter(|g| !g.trim().is_empty())
        .collect()
}

/// What counts when the card describes `{{user}}` differently from the persona.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Name, looks, personality and voice from the persona; role and situation from the card.
    #[default]
    Persona,
    /// The card; the persona only fills in what the card leaves open.
    Card,
}

impl Priority {
    pub fn parse(s: &str) -> Option<Priority> {
        match s {
            "persona" => Some(Priority::Persona),
            "card" => Some(Priority::Card),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Persona => "persona",
            Priority::Card => "card",
        }
    }

    /// The one sentence that closes the persona block.
    fn sentence(self, user_name: &str) -> String {
        match self {
            Priority::Persona => format!(
                "Vorrang: Beschreibt die Figurenkarte {user_name} anders, gelten für Name, Aussehen, Persönlichkeit und Sprechweise die Angaben hier; Rolle, Beziehung und Lage in der Geschichte kommen aus der Karte."
            ),
            Priority::Card => format!(
                "Vorrang: Beschreibt die Figurenkarte {user_name} anders, gilt die Karte; die Angaben hier ergänzen nur, was die Karte offenlässt."
            ),
        }
    }
}

/// What the prompt tells about the figure the user plays (the chat's persona; its name is
/// `user_name`). Every field may be empty; all empty = no block at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserFigure {
    pub pronouns: String,
    pub appearance: String,
    pub personality: String,
    pub background: String,
    /// Free text, after the labelled fields.
    pub description: String,
    /// The block goes to the end of the user's last message on every reply instead of the static
    /// head: the model follows it more closely, at the cost of reading it again each time.
    pub every_turn: bool,
    /// Effective for this chat: the character's override, else the persona's own setting.
    pub priority: Priority,
}

impl UserFigure {
    /// Only a free description (the persona format before the extra fields).
    pub fn plain(description: &str) -> UserFigure {
        UserFigure { description: description.into(), ..UserFigure::default() }
    }

    /// The `# Nutzerfigur` block with macros resolved, or None if every field is empty.
    fn block(&self, user_name: &str, r: impl Fn(&str) -> String) -> Option<String> {
        let pronouns = r(&self.pronouns);
        let pronouns = pronouns.trim();
        let mut body = vec![];
        for (label, text) in [("Aussehen: ", &self.appearance), ("Persönlichkeit: ", &self.personality), ("Hintergrund: ", &self.background), ("", &self.description)] {
            let text = r(text);
            if !text.trim().is_empty() {
                body.push(format!("{label}{}", text.trim()));
            }
        }
        if pronouns.is_empty() && body.is_empty() {
            return None;
        }
        let mut block = format!("# Nutzerfigur: {user_name}");
        if !pronouns.is_empty() {
            block.push_str(&format!(" ({pronouns})"));
        }
        block.push_str(" (gespielt vom Nutzer)");
        for part in body {
            block.push_str(&format!("\n\n{part}"));
        }
        block.push_str(&format!("\n\n{}", self.priority.sentence(user_name)));
        Some(block)
    }
}

/// The static head. `persona` describes the figure the user plays in this chat (all empty: no block).
pub fn head(lang: Language, card: &Card, user_name: &str, persona: &UserFigure) -> String {
    let r = |t: &str| resolve(t, &card.name, user_name);
    let mut parts = vec![r(&BASE.replacen("{{language_rule}}", lang.rule(), 1))];
    if !card.system_prompt.is_empty() {
        parts.push(r(&card.system_prompt));
    }
    let mut figure = format!("# Figur: {}", card.name);
    for (label, text) in [("", &card.description), ("Persönlichkeit: ", &card.personality), ("Szenario: ", &card.scenario)] {
        if !text.is_empty() {
            figure.push_str(&format!("\n\n{label}{}", r(text)));
        }
    }
    parts.push(figure);
    if !card.mes_example.is_empty() {
        let examples = r(&card.mes_example).replace("<START>", "---").replace("<start>", "---");
        parts.push(format!("# Beispieldialoge (nur Stil, nicht Teil der Geschichte)\n\n{examples}"));
    }
    if !persona.every_turn {
        if let Some(block) = persona.block(user_name, r) {
            parts.push(block);
        }
    }
    parts.join("\n\n")
}

/// Per-reply instructions; empty if there are none. The card's come first, then the language of
/// the reply (none for German).
pub fn tail(lang: Language, card: &Card, user_name: &str) -> String {
    notes(lang.note(), None, card, user_name)
}

fn notes(language: Option<&str>, length: Option<&str>, card: &Card, user_name: &str) -> String {
    let mut notes = vec![];
    if !card.post_history_instructions.is_empty() {
        notes.push(resolve(&card.post_history_instructions, &card.name, user_name));
    }
    if let Some(language) = language {
        notes.push(language.to_string());
    }
    if let Some(length) = length {
        notes.push(length.to_string());
    }
    notes.join("\n")
}

/// The full message list for one generation. `history` must end with the user's message.
/// Messages of the same role in a row (e.g. from an imported chat) form one turn, joined by a
/// blank line, so the turns strictly alternate; the stored history stays as it is.
pub fn build(lang: Language, card: &Card, user_name: &str, persona: &UserFigure, history: &[Turn]) -> Vec<ChatMsg> {
    assemble(lang, lang.note(), None, card, user_name, persona, history)
}

/// [`build`] for a reply of the chat: the per-reply content also says how long the reply should
/// be ([`length_note`] for an answer length of `length` tokens). The head and the history are
/// the same as without it, so the cache holds.
#[allow(clippy::too_many_arguments)]
pub fn build_reply(lang: Language, card: &Card, user_name: &str, persona: &UserFigure, history: &[Turn], length: i64) -> Vec<ChatMsg> {
    assemble(lang, lang.note(), length_note(lang, length).as_deref(), card, user_name, persona, history)
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    lang: Language,
    language_note: Option<&str>,
    length_note: Option<&str>,
    card: &Card,
    user_name: &str,
    persona: &UserFigure,
    history: &[Turn],
) -> Vec<ChatMsg> {
    let mut msgs = vec![ChatMsg { role: "system", content: head(lang, card, user_name, persona) }];
    if history.first().is_some_and(|t| t.role == Role::Assistant) {
        msgs.push(ChatMsg { role: "user", content: OPENER.into() });
    }
    for t in history {
        let role = if t.role == Role::User { "user" } else { "assistant" };
        match msgs.last_mut() {
            Some(last) if last.role == role => {
                last.content.push_str("\n\n");
                last.content.push_str(&t.content);
            }
            _ => msgs.push(ChatMsg { role, content: t.content.clone() }),
        }
    }
    // Per-reply content goes to the end of the last user message (layout B).
    let figure = persona.every_turn.then(|| persona.block(user_name, |t| resolve(t, &card.name, user_name))).flatten();
    let tail = notes(language_note, length_note, card, user_name);
    if let Some(last) = msgs.last_mut().filter(|m| m.role == "user") {
        if let Some(figure) = figure {
            last.content.push_str(&format!("\n\n[{figure}]"));
        }
        if !tail.is_empty() {
            last.content.push_str(&format!("\n\n[Anweisung für diese Antwort: {tail}]"));
        }
    }
    msgs
}

/// Strings at which the model must stop because it would start writing the user's turn.
pub fn stop_strings(user_name: &str) -> Vec<String> {
    vec![format!("\n{user_name}:")]
}

/// Tokens one turn of the history costs beyond its text (the template's role markers, the blank
/// line of a joined turn). Generous on purpose: the estimate of a history stays above what the
/// template really renders, so a cut planned with it fits.
pub const TURN_OVERHEAD: usize = 8;

/// The history without any turn, for measuring what every prompt of a chat costs besides its
/// turns: the head, the opener with an empty turn of the character, and an empty user turn that
/// carries the per-reply content (with the length note of [`build_reply`]).
pub fn frame(lang: Language, card: &Card, user_name: &str, persona: &UserFigure, length: i64) -> Vec<ChatMsg> {
    let empty = [Turn { role: Role::Assistant, content: String::new() }, Turn { role: Role::User, content: String::new() }];
    build_reply(lang, card, user_name, persona, &empty, length)
}

/// The part of a chat's history that goes into the prompt when all of it does not fit (layout B,
/// rule 3 of `docs/cache.md`): the first message stays (it sets the scene), then the history from
/// turn `from` on. The turns in between are left out; the stored chat stays as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    /// Index of the oldest turn after the first message that is kept (1: nothing left out).
    pub from: usize,
    /// The first message stays. False only when the first message and the newest turn together do
    /// not fit.
    pub keep_first: bool,
}

impl Cut {
    /// The whole history.
    pub const NONE: Cut = Cut { from: 1, keep_first: true };

    /// How many turns of a history of `len` turns are left out.
    pub fn dropped(&self, len: usize) -> usize {
        self.from.clamp(1, len.max(1)) - 1 + usize::from(!self.keep_first && len > 0)
    }

    /// The turns that go into the prompt.
    pub fn apply(&self, history: &[Turn]) -> Vec<Turn> {
        let first = history.first().filter(|_| self.keep_first);
        let rest = &history[self.from.max(1).min(history.len())..];
        first.into_iter().chain(rest).cloned().collect()
    }
}

/// Where a history is cut so that it fits into `room` tokens; `sizes` holds what each turn costs
/// (its tokens plus [`TURN_OVERHEAD`]). None: not even the newest turn fits.
///
/// The cut is sticky: the chat is played back as it grew, turn by turn, and only when the kept
/// part overflows `room` the oldest turns after the first message are left out, in one block,
/// until a quarter of `room` is free again. Until the room is used up once more the cut stays
/// where it is, so the prompt only grows at its end and the model server keeps reading it from
/// its cache. Rebuilt from the whole history each time (no stored state), it is the same for the
/// same history, room and sizes, also after a restart.
pub fn cut(sizes: &[usize], room: usize) -> Option<Cut> {
    let Some((&first, rest)) = sizes.split_first() else { return Some(Cut::NONE) };
    let block = (room / 4).max(1);
    let (mut from, mut sum) = (1, first);
    for i in 1..sizes.len() {
        sum += sizes[i];
        if sum > room {
            while from < i && sum + block > room {
                sum -= sizes[from];
                from += 1;
            }
        }
    }
    if sum <= room {
        return Some(Cut { from, keep_first: true });
    }
    // Only the first message and the newest turn are left, and both do not fit: the newest alone.
    match rest.last() {
        Some(&newest) if newest <= room => Some(Cut { from: sizes.len() - 1, keep_first: false }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Card {
        Card {
            name: "Mara".into(),
            description: "{{char}} keeps the lighthouse. {{user}} is a guest.".into(),
            personality: "dry".into(),
            scenario: "Storm.".into(),
            first_mes: "\"Evening, {{User}}.\"".into(),
            mes_example: "<START>\n{{user}}: Hi\n{{char}}: Hm.".into(),
            system_prompt: "{{original}}Keep replies short.".into(),
            post_history_instructions: "{{char}} stays calm.".into(),
            ..Card::default()
        }
    }

    fn none() -> UserFigure {
        UserFigure::default()
    }

    /// Golden texts of the priority sentence (`Priority::Persona` / `Priority::Card`).
    fn persona_first(user: &str) -> String {
        format!("Vorrang: Beschreibt die Figurenkarte {user} anders, gelten für Name, Aussehen, Persönlichkeit und Sprechweise die Angaben hier; Rolle, Beziehung und Lage in der Geschichte kommen aus der Karte.")
    }

    fn card_first(user: &str) -> String {
        format!("Vorrang: Beschreibt die Figurenkarte {user} anders, gilt die Karte; die Angaben hier ergänzen nur, was die Karte offenlässt.")
    }

    fn turn(role: Role, content: &str) -> Turn {
        Turn { role, content: content.into() }
    }

    #[test]
    fn macros() {
        assert_eq!(resolve("{{char}} & {{ USER }} & <BOT> & <USER>", "Mara", "Lukas"), "Mara & Lukas & Mara & Lukas");
        assert_eq!(resolve("{{random:a,b}} {{char", "Mara", "Lukas"), "{{random:a,b}} {{char");
        assert_eq!(resolve("ä{{original}}ö", "Mara", "Lukas"), "äö");
    }

    #[test]
    fn head_layout() {
        let h = head(Language::De, &card(), "Lukas", &none());
        assert!(h.starts_with("Du schreibst ein fiktives Rollenspiel"));
        assert!(h.contains("Du spielst Mara sowie"));
        assert!(h.contains("\n\nKeep replies short.\n\n# Figur: Mara\n\nMara keeps the lighthouse. Lukas is a guest.\n\nPersönlichkeit: dry\n\nSzenario: Storm.\n\n# Beispieldialoge"));
        assert!(h.contains("---\nLukas: Hi\nMara: Hm."));
        assert!(!h.contains("{{"));
    }

    /// Golden: the persona block sits in the static head after the card (and its examples) and
    #[test]
    fn persona_block_in_the_head() {
        let persona = "{{user}} ist Kapitän und kennt {{char}} seit Jahren.";
        let h = head(Language::De, &card(), "Lukas", &UserFigure::plain(persona));
        let expected = format!(
            "---\nLukas: Hi\nMara: Hm.\n\n# Nutzerfigur: Lukas (gespielt vom Nutzer)\n\nLukas ist Kapitän und kennt Mara seit Jahren.\n\n{}",
            persona_first("Lukas")
        );
        assert!(h.ends_with(&expected), "{h}");
        assert!(h.contains("Der Nutzer spielt Lukas."));
        let msgs = build(Language::De, &card(), "Lukas", &UserFigure::plain(persona), &[turn(Role::User, "Hi")]);
        assert_eq!(msgs[0].content, h, "the block lives in the system message only");
        assert!(!msgs[1].content.contains("Kapitän"));

        for empty in ["", "  \n "] {
            let plain = head(Language::De, &card(), "Lukas", &UserFigure::plain(empty));
            assert!(!plain.contains("Nutzerfigur"));
            assert!(plain.ends_with(&format!("Mara: Hm.")), "no block, no blank lines: {plain}");
        }
    }

    /// Golden: pronouns go into the heading, then only the filled fields in a fixed order, then
    /// the free description; macros resolved everywhere.
    #[test]
    fn persona_block_with_fields_and_pronouns() {
        let figure = UserFigure {
            pronouns: " sie/ihr ".into(),
            appearance: "Groß, Narbe über dem Auge.".into(),
            personality: "  ".into(),
            background: "Kennt {{char}} vom Hafen.".into(),
            description: "{{user}} schweigt gern.".into(),
            every_turn: false,
            priority: Priority::Persona,
        };
        let h = head(Language::De, &card(), "Bea", &figure);
        let expected = format!(
            "Mara: Hm.\n\n# Nutzerfigur: Bea (sie/ihr) (gespielt vom Nutzer)\n\nAussehen: Groß, Narbe über dem Auge.\n\nHintergrund: Kennt Mara vom Hafen.\n\nBea schweigt gern.\n\n{}",
            persona_first("Bea")
        );
        assert!(h.ends_with(&expected), "{h}");

        let only_pronouns = UserFigure { pronouns: "er/ihm".into(), ..UserFigure::default() };
        assert!(head(Language::De, &card(), "Bo", &only_pronouns).ends_with(&format!("Mara: Hm.\n\n# Nutzerfigur: Bo (er/ihm) (gespielt vom Nutzer)\n\n{}", persona_first("Bo"))));
        let personality_only = UserFigure { personality: "{{user}} lacht viel.".into(), ..UserFigure::default() };
        assert!(head(Language::De, &card(), "Bo", &personality_only).contains("# Nutzerfigur: Bo (gespielt vom Nutzer)\n\nPersönlichkeit: Bo lacht viel.\n\n"));
    }

    /// Every field empty or blank: no block, the head is the same as without a persona.
    #[test]
    fn empty_persona_fields_add_no_block() {
        let blank = UserFigure { pronouns: " ".into(), appearance: "\n".into(), personality: "".into(), background: "  ".into(), description: "".into(), every_turn: false, priority: Priority::Card };
        assert_eq!(head(Language::De, &card(), "Bea", &blank), head(Language::De, &card(), "Bea", &none()));
        assert!(!head(Language::De, &card(), "Bea", &blank).contains("Nutzerfigur"));
    }

    /// Golden: exactly one priority sentence closes the block, for either value; the value the
    /// server passes in is already the effective one (character override ?? persona setting).
    #[test]
    fn priority_sentence_closes_the_block() {
        let fig = |priority| UserFigure { appearance: "Rote Haare.".into(), priority, ..UserFigure::default() };
        for (priority, sentence) in [(Priority::Persona, persona_first("Bea")), (Priority::Card, card_first("Bea"))] {
            let h = head(Language::De, &card(), "Bea", &fig(priority));
            let expected = format!("Mara: Hm.\n\n# Nutzerfigur: Bea (gespielt vom Nutzer)\n\nAussehen: Rote Haare.\n\n{sentence}");
            assert!(h.ends_with(&expected), "{h}");
            assert_eq!(h.matches("Vorrang:").count(), 1);
            let every = UserFigure { every_turn: true, ..fig(priority) };
            let last = build(Language::De, &card(), "Bea", &every, &[turn(Role::User, "Na?")]).pop().unwrap().content;
            assert!(last.starts_with(&format!("Na?\n\n[# Nutzerfigur: Bea (gespielt vom Nutzer)\n\nAussehen: Rote Haare.\n\n{sentence}]")), "{last}");
        }
        // No block, no sentence.
        let blank = UserFigure { priority: Priority::Card, ..UserFigure::default() };
        assert!(!head(Language::De, &card(), "Bea", &blank).contains("Vorrang:"));
        assert_eq!((Priority::parse("card"), Priority::parse("persona"), Priority::parse("x")), (Some(Priority::Card), Some(Priority::Persona), None));
        assert_eq!(serde_json::to_value(Priority::Card).unwrap(), "card");
    }

    #[test]
    fn persona_every_turn_goes_to_the_last_user_message() {
        let fig = UserFigure { pronouns: "sie/ihr".into(), description: "{{user}} kennt {{char}}.".into(), every_turn: true, ..UserFigure::default() };
        let h = head(Language::De, &card(), "Bea", &fig);
        assert!(!h.contains("Nutzerfigur"), "not in the static head");
        let msgs = build(Language::De, &card(), "Bea", &fig, &[turn(Role::User, "Hi"), turn(Role::Assistant, "Hm."), turn(Role::User, "Na?")]);
        assert_eq!(msgs[0].content, h);
        assert_eq!(msgs[1].content, "Hi", "earlier turns stay as they are (cache)");
        let last = &msgs.last().unwrap().content;
        assert!(last.starts_with(&format!("Na?\n\n[# Nutzerfigur: Bea (sie/ihr) (gespielt vom Nutzer)\n\nBea kennt Mara.\n\n{}]", persona_first("Bea"))), "{last}");
        assert!(last.ends_with("[Anweisung für diese Antwort: Mara stays calm.]"), "{last}");
        // All empty: nothing appended.
        let blank = UserFigure { every_turn: true, ..UserFigure::default() };
        assert_eq!(build(Language::De, &card(), "Bea", &blank, &[turn(Role::User, "Na?")]).last().unwrap().content, "Na?\n\n[Anweisung für diese Antwort: Mara stays calm.]");
    }




    #[test]
    fn messages_layout_b() {
        let history = [
            turn(Role::Assistant, "\"Evening, Lukas.\""),
            turn(Role::User, "Hello."),
            turn(Role::Assistant, "She nods."),
            turn(Role::User, "Nice storm."),
        ];
        let msgs = build(Language::De, &card(), "Lukas", &none(), &history);
        let roles: Vec<_> = msgs.iter().map(|m| m.role).collect();
        assert_eq!(roles, ["system", "user", "assistant", "user", "assistant", "user"]);
        assert_eq!(msgs[1].content, OPENER);
        assert_eq!(msgs[3].content, "Hello.", "older user turns carry no tail");
        assert_eq!(msgs[5].content, "Nice storm.\n\n[Anweisung für diese Antwort: Mara stays calm.]");
    }

    /// The cache only works if everything before the newest turn is byte-identical next time.
    #[test]
    fn prefix_is_stable_across_turns() {
        let mut history = vec![turn(Role::Assistant, "Hi."), turn(Role::User, "One.")];
        let first = build(Language::De, &card(), "Lukas", &none(), &history);
        history.push(turn(Role::Assistant, "Reply."));
        history.push(turn(Role::User, "Two."));
        let second = build(Language::De, &card(), "Lukas", &none(), &history);
        let stable = first.len() - 1; // all but the last user message (it carried the tail)
        assert_eq!(first[..stable], second[..stable]);
        assert_eq!(second[stable].content, "One.");
    }

    /// Messages of the same role in a row (e.g. from an imported chat) form one turn for the
    /// template, joined by a blank line, so the turns strictly alternate.
    #[test]
    fn same_role_in_a_row_is_one_turn() {
        let history = [
            turn(Role::Assistant, "Hi."),
            turn(Role::Assistant, "Still there?"),
            turn(Role::User, "Yes."),
            turn(Role::User, "Sorry."),
        ];
        let msgs = build(Language::De, &card(), "Lukas", &none(), &history);
        let got: Vec<(&str, &str)> = msgs[1..].iter().map(|m| (m.role, m.content.as_str())).collect();
        assert_eq!(
            got,
            [("user", OPENER), ("assistant", "Hi.\n\nStill there?"), ("user", "Yes.\n\nSorry.\n\n[Anweisung für diese Antwort: Mara stays calm.]")]
        );
    }

    /// One string per prompt as a template lays it out (role marker, then the text).
    fn rendered(msgs: &[ChatMsg]) -> String {
        msgs.iter().map(|m| format!("<{}>{}", m.role, m.content)).collect()
    }

    /// Layout B with turns joined: whatever comes next (a reply and a user message, or another
    /// user message right after one), the prompt so far stays the start of the next one, byte for
    /// byte (without per-reply content, which sits at the very end).
    #[test]
    fn joined_turns_keep_the_prefix() {
        let quiet = Card { post_history_instructions: String::new(), ..card() };
        let mut history = vec![turn(Role::Assistant, "Hi."), turn(Role::Assistant, "Hello?"), turn(Role::User, "One.")];
        let first = build(Language::De, &quiet, "Lukas", &none(), &history);
        history.push(turn(Role::User, "Two."));
        let second = build(Language::De, &quiet, "Lukas", &none(), &history);
        assert!(rendered(&second).starts_with(&rendered(&first)), "{}", rendered(&second));
        assert_eq!(second.last().unwrap().content, "One.\n\nTwo.");
        history.extend([turn(Role::Assistant, "Reply."), turn(Role::User, "Three.")]);
        let third = build(Language::De, &quiet, "Lukas", &none(), &history);
        assert!(rendered(&third).starts_with(&rendered(&second)));
        assert_eq!(third[..second.len()], second[..]);
    }

    /// A history without two of the same role in a row: exactly one message per turn, as before.
    #[test]
    fn alternating_history_is_unchanged() {
        let history = [turn(Role::User, "A"), turn(Role::Assistant, "B"), turn(Role::User, "C"), turn(Role::Assistant, "D"), turn(Role::User, "E")];
        let msgs = build(Language::De, &card(), "Lukas", &none(), &history);
        assert_eq!(msgs.len(), 1 + history.len());
        assert_eq!(msgs[0].content, head(Language::De, &card(), "Lukas", &none()));
        for (m, t) in msgs[1..msgs.len() - 1].iter().zip(&history) {
            assert_eq!((m.role == "user", m.content.as_str()), (t.role == Role::User, t.content.as_str()));
        }
        assert_eq!(msgs.last().unwrap().content, format!("E\n\n[Anweisung für diese Antwort: Mara stays calm.]"));
    }

    /// The line of the rules about the language as it was before the app had a language setting.
    const GERMAN_LINE: &str = "- Schreibe in der Sprache, in der der Nutzer schreibt (Standard: Deutsch).";

    /// German is the default and changes nothing: the head still holds the old line, and no
    /// language note joins the per-reply content.
    #[test]
    fn german_leaves_the_prompt_as_it_was() {
        assert_eq!(Language::default(), Language::De);
        let h = head(Language::De, &card(), "Lukas", &none());
        assert!(h.contains(&format!("\n{GERMAN_LINE}")), "{h}");
        assert!(!h.contains("{{") && !h.contains("language_rule"));
        assert_eq!(tail(Language::De, &card(), "Lukas"), "Mara stays calm.");
        let quiet = Card { post_history_instructions: String::new(), ..card() };
        assert_eq!(build(Language::De, &quiet, "Lukas", &none(), &[turn(Role::User, "Hallo.")]).last().unwrap().content, "Hallo.", "nothing appended");
    }

    /// Any other language is asked for twice (layout B): one fixed line in the static head in
    /// place of the German one, and a short note at the end of the last user message.
    #[test]
    fn another_language_is_asked_for_in_the_head_and_per_reply() {
        let german = head(Language::De, &card(), "Lukas", &none());
        for (lang, in_head, note) in [
            (Language::En, "Always write your replies in English.", "Reply in English."),
            (Language::Es, "Escribe siempre tus respuestas en español.", "Responde en español."),
            (Language::Ru, "Всегда пиши свои ответы на русском языке.", "Отвечай на русском языке."),
        ] {
            let h = head(lang, &card(), "Lukas", &none());
            assert!(h.contains(in_head) && !h.contains(GERMAN_LINE), "{h}");
            assert_eq!(h.matches(in_head).count(), 1);
            // Only that one line differs from the German head.
            let differing: Vec<(&str, &str)> = german.lines().zip(h.lines()).filter(|(a, b)| a != b).collect();
            assert_eq!(differing.len(), 1, "{differing:?}");
            assert_eq!(german.lines().count(), h.lines().count());

            let history = [turn(Role::Assistant, "Hi."), turn(Role::User, "Eins."), turn(Role::Assistant, "Hm."), turn(Role::User, "Zwei.")];
            let msgs = build(lang, &card(), "Lukas", &none(), &history);
            assert_eq!(msgs[0].content, h);
            assert_eq!(msgs[3].content, "Eins.", "older user turns carry no note (cache)");
            assert_eq!(msgs.last().unwrap().content, format!("Zwei.\n\n[Anweisung für diese Antwort: Mara stays calm.\n{note}]"));
            // Without anything from the card the note stands alone.
            let quiet = Card { post_history_instructions: String::new(), ..card() };
            assert_eq!(build(lang, &quiet, "Lukas", &none(), &history).last().unwrap().content, format!("Zwei.\n\n[Anweisung für diese Antwort: {note}]"));
        }
        assert_eq!(Language::ALL.map(Language::as_str), ["de", "en", "es", "ru"]);
        assert_eq!((Language::parse("ru"), Language::parse("EN"), Language::parse("")), (Some(Language::Ru), None, None));
        assert_eq!(serde_json::to_value(Language::Es).unwrap(), "es");
    }


    /// The prefix stays byte-identical across turns in every language (the cache).
    #[test]
    fn prefix_is_stable_in_every_language() {
        for lang in Language::ALL {
            let mut history = vec![turn(Role::Assistant, "Hi."), turn(Role::User, "One.")];
            let first = build(lang, &card(), "Lukas", &none(), &history);
            history.extend([turn(Role::Assistant, "Reply."), turn(Role::User, "Two.")]);
            let second = build(lang, &card(), "Lukas", &none(), &history);
            let stable = first.len() - 1;
            assert_eq!(first[..stable], second[..stable], "{lang:?}");
            assert_eq!(second[stable].content, "One.");
        }
    }

    /// `first_mes` is the first greeting, the alternate greetings follow in their order, macros
    /// resolved and empty ones left out; without `first_mes` the first alternate one opens.
    #[test]
    fn greetings_are_first_mes_then_the_alternate_ones() {
        let c = Card { alternate_greetings: vec!["Morning, {{user}}.".into(), "  ".into(), "<BOT> waves.".into()], ..card() };
        assert_eq!(greetings(&c, "Lukas"), ["\"Evening, Lukas.\"", "Morning, Lukas.", "Mara waves."]);
        let no_first = Card { first_mes: " ".into(), ..c.clone() };
        assert_eq!(greetings(&no_first, "Lukas"), ["Morning, Lukas.", "Mara waves."]);
        assert!(greetings(&Card { name: "X".into(), ..Card::default() }, "U").is_empty());
    }

    /// A chat that grows past its room is cut from its oldest end in blocks: the first message
    /// stays, a quarter of the room is freed at once, and between two cuts the prompt only grows
    /// at its end (the same messages, byte for byte, as the turn before).
    #[test]
    fn a_long_chat_is_cut_from_the_oldest_end_in_blocks() {
        let room = 2000;
        let text = |i: usize| format!("Runde {i}: {}", "x".repeat(30 + i % 7 * 10));
        let mut history = vec![turn(Role::Assistant, "Willkommen im Leuchtturm.")];
        let size = |t: &Turn| t.content.len() / 4 + TURN_OVERHEAD;
        let (mut cuts, mut last_msgs): (Vec<Cut>, Vec<ChatMsg>) = (vec![], vec![]);
        for i in 0..400 {
            history.push(turn(if i % 2 == 0 { Role::User } else { Role::Assistant }, &text(i)));
            if history.last().unwrap().role != Role::User {
                continue; // a prompt is built when the user has written
            }
            let sizes: Vec<usize> = history.iter().map(size).collect();
            let c = cut(&sizes, room).unwrap();
            let kept = c.apply(&history);
            assert!(kept.iter().map(size).sum::<usize>() <= room, "fits the room");
            assert!(c.keep_first && kept[0] == history[0], "the first message stays");
            assert_eq!(kept.last(), history.last(), "the newest turn is always in");
            assert_eq!(kept.len() + c.dropped(history.len()), history.len());
            let msgs = build(Language::De, &card(), "Lukas", &none(), &kept);
            if let Some(prev) = cuts.last() {
                assert!(c.from >= prev.from, "the cut only moves forward");
                if c == *prev {
                    let stable = last_msgs.len() - 1; // the last user message carried the tail
                    assert_eq!(msgs[..stable], last_msgs[..stable], "same cut: the prompt grows at its end only");
                } else {
                    let freed: usize = sizes[prev.from..c.from].iter().sum();
                    assert!(freed >= room / 4, "a block of at least a quarter of the room goes at once");
                }
            }
            cuts.push(c);
            last_msgs = msgs;
        }
        let distinct = cuts.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(cuts[0] == Cut::NONE && cuts.last().unwrap().from > 1, "{cuts:?}");
        assert!(distinct * 4 < cuts.len(), "cut rarely: {distinct} moves in {} turns", cuts.len());
        // The same history gives the same cut (no stored state).
        let sizes: Vec<usize> = history.iter().map(size).collect();
        assert_eq!(cut(&sizes, room), cut(&sizes, room));
    }


    /// The first message goes only when it does not fit together with the newest turn; when the
    /// newest turn alone does not fit, there is no cut (the server answers `context_full`).
    #[test]
    fn the_newest_turn_must_fit() {
        assert_eq!(cut(&[], 10), Some(Cut::NONE));
        assert_eq!(cut(&[5, 3, 4], 100), Some(Cut::NONE), "everything fits: nothing is cut");
        assert_eq!(cut(&[60, 10, 50], 100), Some(Cut { from: 2, keep_first: false }));
        assert_eq!(cut(&[20, 90, 10, 50], 100), Some(Cut { from: 2, keep_first: true }), "the big turn went when it overflowed");
        assert_eq!(cut(&[20, 10, 101], 100), None);
        assert_eq!(cut(&[101], 100), None, "a first message that is also the newest");
        assert_eq!(Cut { from: 2, keep_first: false }.dropped(3), 2);
        assert_eq!(Cut::NONE.dropped(0), 0);
        let history = [turn(Role::Assistant, "A"), turn(Role::User, "B"), turn(Role::User, "C")];
        assert_eq!(Cut { from: 2, keep_first: false }.apply(&history), [turn(Role::User, "C")]);
        assert_eq!(Cut::NONE.apply(&history), history);
    }

    /// The frame is the head plus empty turns: the opener, the per-reply content, nothing else.
    #[test]
    fn the_frame_holds_the_head_and_the_tail() {
        let f = frame(Language::En, &card(), "Lukas", &none(), 150);
        let got: Vec<(&str, &str)> = f[1..].iter().map(|m| (m.role, m.content.as_str())).collect();
        assert_eq!(f[0].content, head(Language::En, &card(), "Lukas", &none()));
        let length = length_note(Language::En, 150).unwrap();
        let tail = format!("\n\n[Anweisung für diese Antwort: Mara stays calm.\nReply in English.\n{length}]");
        assert_eq!(got, [("user", OPENER), ("assistant", ""), ("user", tail.as_str())]);
    }

    /// The answer length as words in the reply's language: about 85 % of it, at most the length
    /// itself (the hard cap), a clean end asked for; nothing for a length that is only an upper bound.
    #[test]
    fn the_length_note_names_words_in_the_language() {
        assert_eq!(
            length_note(Language::De, 150).as_deref(),
            Some("Länge: etwa 75 Wörter, höchstens 90. Hör mit einem vollständigen Satz auf, nicht mitten im Gedanken.")
        );
        assert_eq!(
            length_note(Language::En, 150).as_deref(),
            Some("Length: about 90 words, never more than 110. Finish with a complete sentence; do not stop mid-thought.")
        );
        assert_eq!(length_note(Language::En, 400).as_deref(), Some("Length: about 240 words, never more than 280. Finish with a complete sentence; do not stop mid-thought."));
        assert_eq!(length_note(Language::En, 800).as_deref(), Some("Length: about 480 words, never more than 560. Finish with a complete sentence; do not stop mid-thought."));
        assert!(length_note(Language::Es, 150).unwrap().starts_with("Extensión: unas 85 palabras, nunca más de 100."));
        assert!(length_note(Language::Ru, 150).unwrap().starts_with("Объём: около 70 слов, не больше 85."));
        assert!(length_note(Language::En, 1).unwrap().starts_with("Length: about 5 words, never more than 10."), "never less than 5 words");
        assert!(length_note(Language::De, crate::length::NOTE_MAX_TOKENS).is_some());
        assert_eq!(length_note(Language::De, crate::length::NOTE_MAX_TOKENS + 1), None, "only an upper bound");
    }

    /// The length note is per-reply content (layout B): last in the final user message before the
    /// one turn to the next and a reply's prompt differs from `build` only in its tail.
    #[test]
    fn the_length_note_sits_at_the_end_of_the_last_user_message() {
        let mut history = vec![turn(Role::Assistant, "Hi."), turn(Role::User, "Eins.")];
        let first = build_reply(Language::De, &card(), "Lukas", &none(), &history, 150);
        history.extend([turn(Role::Assistant, "Hm."), turn(Role::User, "Zwei.")]);
        let second = build_reply(Language::De, &card(), "Lukas", &none(), &history, 150);
        let note = length_note(Language::De, 150).unwrap();
        assert_eq!(first[0].content, second[0].content, "the head is the same in every turn");
        assert_eq!(first[0].content, head(Language::De, &card(), "Lukas", &none()));
        assert!(!first[0].content.contains("Länge:"));
        assert_eq!(second[3].content, "Eins.", "older user turns carry no note (cache)");
        assert_eq!(second.last().unwrap().content, format!("Zwei.\n\n[Anweisung für diese Antwort: Mara stays calm.\n{note}]"));
        // Without the length the same prompt, only the note is missing.
        let plain = build(Language::De, &card(), "Lukas", &none(), &history);
        assert_eq!(plain[..plain.len() - 1], second[..second.len() - 1]);
        assert_eq!(plain.last().unwrap().content, format!("Zwei.\n\n[Anweisung für diese Antwort: Mara stays calm.]"));
        // Order with every note: card, language, then reply length.
        let en = build_reply(Language::En, &card(), "Lukas", &none(), &history, 400);
        let en_note = length_note(Language::En, 400).unwrap();
        assert!(en.last().unwrap().content.ends_with(&format!("[Anweisung für diese Antwort: Mara stays calm.\nReply in English.\n{en_note}]")));
        // A length that is only an upper bound adds nothing.
        assert_eq!(build_reply(Language::De, &card(), "Lukas", &none(), &history, 4096), plain);
    }

    #[test]
    fn no_opener_when_user_starts() {
        let msgs = build(Language::De, &Card { name: "X".into(), ..Card::default() }, "U", &none(), &[turn(Role::User, "Hi")]);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].content, "Hi");
    }
}
