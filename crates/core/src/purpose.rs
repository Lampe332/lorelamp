//! What the user wants to do with a model, read from a sentence ("Ich will richtig lange Chats mit
//! 3–4 User-Charakteren", German or English): a fixed list of needs, found by words and short
//! phrases, each with the words that led to it. No model is asked: the same sentence always gives
//! the same needs, and the UI shows them as chips the user can correct.
//!
//! The needs steer two things: which catalog models are suggested (`crates/server/src/fetch/suggest.rs`)
//! and which settings "Beste Einstellung finden" proposes: which measured row of a benchmark
//! ([`bench_purpose`]) and which sampler values ([`sampler_for`]).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bench;
use crate::sampler::Sampler;

/// One thing the user wants. The order is the order of the chips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// Chats that go on for hundreds of messages.
    LongChats,
    /// Several characters in one chat (a group chat, a party).
    Group,
    /// Narration, adventures, world building.
    Story,
    German,
    English,
    /// Quick answers.
    Fast,
    /// Much context: long cards, lorebooks, "it should not forget".
    Context,
    /// Models that rarely or never refuse.
    Uncensored,
    /// A weak PC: no graphics card, little memory, a laptop.
    WeakPc,
    /// The best writing the PC can run, speed second.
    Quality,
    ShortReplies,
    LongReplies,
}

impl Need {
    pub const ALL: [Need; 12] = [
        Need::LongChats,
        Need::Group,
        Need::Story,
        Need::German,
        Need::English,
        Need::Fast,
        Need::Context,
        Need::Uncensored,
        Need::WeakPc,
        Need::Quality,
        Need::ShortReplies,
        Need::LongReplies,
    ];
}

/// Why a need was found: the words of the sentence (lower case, umlauts written out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hit {
    pub need: Need,
    pub phrase: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Parsed {
    /// In the order of [`Need::ALL`], each once.
    pub needs: Vec<Need>,
    /// How many characters the sentence names for a group ("3–4 Charaktere" → 4).
    pub figures: Option<u8>,
    pub hits: Vec<Hit>,
}

/// A stored purpose: the sentence and the needs (parsed, or corrected by the user).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Purpose {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub needs: Vec<Need>,
    #[serde(default)]
    pub figures: Option<u8>,
}

impl Purpose {
    pub fn from_text(text: &str) -> Purpose {
        let p = parse(text);
        Purpose { text: text.trim().to_string(), needs: p.needs, figures: p.figures }
    }

    /// The sentence with needs chosen by hand (each once, in chip order); the number of
    /// characters still comes from the sentence.
    pub fn with_needs(text: &str, needs: &[Need]) -> Purpose {
        let needs = Need::ALL.into_iter().filter(|n| needs.contains(n)).collect();
        Purpose { text: text.trim().to_string(), needs, figures: parse(text).figures }
    }

    pub fn has(&self, need: Need) -> bool {
        self.needs.contains(&need)
    }
}

/// Lower case, umlauts written out, everything but letters, digits, `+` and `&` becomes a space.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_alphanumeric() || c == '+' || c == '&' => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Words that turn the next need off ("kein Deutsch", "not fast").
const NEGATORS: [&str; 12] = ["kein", "keine", "keinen", "keiner", "nicht", "not", "no", "without", "ohne", "weder", "nie", "never"];
/// Words that may stand between a negator and the need ("nicht so schnell", "not in German").
const FILLERS: [&str; 16] = ["so", "sehr", "zu", "unbedingt", "very", "too", "really", "a", "any", "ein", "eine", "einen", "in", "auf", "viel", "much"];
/// Said after a need: it does not matter ("Tempo ist egal").
const DISMISSALS: [&str; 9] = ["egal", "unwichtig", "nicht wichtig", "nicht noetig", "nebensache", "not important", "doesn t matter", "does not matter", "not needed"];

/// Phrases per need. A phrase starts at a word and may go on with more letters ("schnell" finds
/// "schnelle"); with `$` at its end the last word must end there ("rp$" does not find "rpg").
const PHRASES: &[(Need, &[&str])] = &[
    (
        Need::LongChats,
        &[
            "hunderte nachrichten", "tausende nachrichten", "viele nachrichten", "hundreds of messages", "thousands of messages", "many messages", "langzeit", "long term", "longterm",
            "ueber wochen", "ueber monate", "ueber tage", "for hours", "for weeks", "for months", "stundenlang", "endlos", "endless", "marathon", "kampagne", "campaign",
        ],
    ),
    (
        Need::Group,
        &[
            "gruppenchat", "gruppen chat", "gruppe", "group", "mehrere figuren", "mehrere charaktere", "mehreren figuren", "mehreren charakteren", "mehrere personen", "mehreren personen", "viele figuren",
            "viele charaktere", "multiple characters", "several characters", "many characters", "multi character", "party$", "ensemble", "mehrere npc", "multiple npc",
        ],
    ),
    (
        Need::Story,
        &[
            "erzaehl", "geschichte", "story", "stories", "abenteuer", "adventure", "narrat", "roman$", "romane$", "novel", "dungeon", "quest", "weltenbau", "worldbuilding", "world building", "pen & paper",
            "pen and paper", "dnd$", "d&d", "rpg$", "spielleiter", "game master", "fantasy",
        ],
    ),
    (Need::German, &["deutsch", "german"]),
    (Need::English, &["englisch", "english"]),
    (
        Need::Fast,
        &["schnell", "flott", "zuegig", "rasch", "tempo$", "fast$", "faster", "quick", "speed", "sofort", "ohne warten", "ohne zu warten", "ohne wartezeit", "instant", "responsive", "snappy", "low latency", "fluessig"],
    ),
    (
        Need::Context,
        &[
            "kontext", "context", "128k", "64k", "32k", "lorebook", "lorebuch", "weltinfo", "world info", "gedaechtnis", "memory", "erinner", "remember", "vergisst", "vergessen", "forget", "lange karten",
            "grosse karten", "long cards",
        ],
    ),
    (
        Need::Uncensored,
        &[
            "unzensiert", "uncensored", "ohne zensur", "keine zensur", "nicht zensiert", "no censorship", "unfiltered", "ungefiltert", "nsfw", "erp$", "erotik", "erotisch", "erotic", "adult", "erwachsen",
            "ohne ablehnung", "keine ablehnung", "no refusal", "lehnt nicht ab", "lehnt nichts ab", "never refuses", "ohne filter", "kein filter", "no filter", "gewalt", "violence", "gore$", "tabu",
        ],
    ),
    (
        Need::WeakPc,
        &[
            "schwacher pc", "schwachen pc", "schwacher rechner", "schwachen rechner", "schwache hardware", "schwacher hardware", "alter pc", "alten pc", "alter rechner", "alten rechner", "laptop", "notebook",
            "ohne grafikkarte", "keine grafikkarte", "no gpu", "without a gpu", "without gpu", "cpu only", "nur cpu", "nur prozessor", "wenig ram", "wenig vram", "wenig speicher", "wenig arbeitsspeicher",
            "wenig grafikspeicher", "low end", "lowend", "potato", "weak pc", "old pc", "weak hardware", "slow pc", "langsamer pc", "langsamen pc", "kleines modell", "kleine modelle", "small model", "igpu$",
            "integrated graphics", "onboard grafik", "buero pc", "office pc",
        ],
    ),
    (
        Need::Quality,
        &[
            "beste qualitaet", "best quality", "hohe qualitaet", "high quality", "qualitaet", "quality", "hochwertig", "anspruchsvoll", "kluegste", "klug", "smart", "intelligent", "schlau", "literarisch",
            "literary", "nuanc", "tiefgruendig", "bestes modell", "best model", "beste modell", "komplexe", "complex",
        ],
    ),
    (Need::ShortReplies, &["kurze antwort", "kurzen antwort", "knappe antwort", "knappen antwort", "short repl", "short answer", "short response", "brief repl", "kurz und knapp", "einzeiler", "one liner"]),
    (
        Need::LongReplies,
        &["lange antwort", "langen antwort", "ausfuehrliche antwort", "ausfuehrlichen antwort", "ausfuehrlich", "long repl", "long answer", "long response", "detailed repl", "detailliert", "detailreich", "viel text"],
    ),
];

/// "lang/long …" within three words of one of these means long chats ("richtig lange Chats").
const LONG_STEMS: [&str; 6] = ["lang", "laenger", "long", "lengthy", "ewig", "riesig"];
const CHAT_STEMS: [&str; 12] = ["chat", "gespraech", "unterhaltung", "rollenspiel", "rp", "session", "sitzung", "conversation", "roleplay", "role", "story", "geschichte"];
/// A number before one of these (within three words) is a group ("4 Charaktere").
const FIGURE_STEMS: [&str; 12] = ["charakter", "character", "figur", "persona", "personen", "people", "leute", "npc", "spieler", "player", "held", "hero"];
const NUMBER_WORDS: [(&str, u8); 14] =
    [("zwei", 2), ("drei", 3), ("vier", 4), ("fuenf", 5), ("sechs", 6), ("sieben", 7), ("acht", 8), ("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6), ("seven", 7), ("eight", 8)];
/// German "fast" (almost) before these is not about speed.
const FAST_IS_ALMOST: [&str; 10] = ["nur", "alle", "alles", "immer", "nie", "jede", "jeden", "kein", "keine", "gar"];

/// Byte ranges of the words of a normalized text.
fn words(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut at = 0;
    for w in text.split(' ') {
        if !w.is_empty() {
            out.push((at, w));
        }
        at += w.len() + 1;
    }
    out
}

/// The need found at word `i` is switched off: a negator right before it (or before a filler), or
/// a dismissal right after the phrase that ends at byte `end`.
fn negated(ws: &[(usize, &str)], i: usize, text: &str, end: usize) -> bool {
    let before = |k: usize| i.checked_sub(k).map(|j| ws[j].1);
    let neg = |w: Option<&str>| w.is_some_and(|w| NEGATORS.contains(&w));
    if neg(before(1)) || (before(1).is_some_and(|w| FILLERS.contains(&w)) && neg(before(2))) {
        return true;
    }
    let tail: String = text[end.min(text.len())..].chars().take(28).collect();
    DISMISSALS.iter().any(|d| tail.trim_start().starts_with(d) || tail.trim_start().strip_prefix("ist ").or_else(|| tail.trim_start().strip_prefix("is ")).is_some_and(|t| t.starts_with(d)))
}

/// Reads the needs out of a sentence. Unknown words are ignored; an empty text has no needs.
pub fn parse(text: &str) -> Parsed {
    let norm = normalize(text);
    let ws = words(&norm);
    let mut hits: Vec<Hit> = Vec::new();
    let mut add = |need: Need, phrase: &str| {
        if !hits.iter().any(|h| h.need == need) {
            hits.push(Hit { need, phrase: phrase.to_string() });
        }
    };

    for (need, phrases) in PHRASES {
        for raw in *phrases {
            let (pat, exact) = match raw.strip_suffix('$') {
                Some(p) => (p, true),
                None => (*raw, false),
            };
            for (i, (start, _)) in ws.iter().enumerate() {
                let rest = &norm[*start..];
                if !rest.starts_with(pat) {
                    continue;
                }
                let end = start + pat.len();
                let ends_word = norm[end..].chars().next().is_none_or(|c| c == ' ');
                if exact && !ends_word {
                    continue;
                }
                if *raw == "fast$" && ws.get(i + 1).is_some_and(|w| FAST_IS_ALMOST.contains(&w.1)) {
                    continue;
                }
                // The phrase with the rest of its last word ("schnell" → "schnelle").
                let word_end = norm[end..].find(' ').map_or(norm.len(), |n| end + n);
                // The phrases of "uncensored" and "weak PC" carry their own "ohne/kein".
                if negated(&ws, i, &norm, word_end) {
                    continue;
                }
                add(*need, &norm[*start..word_end]);
            }
        }
    }

    // "lange Chats", "long roleplay sessions", "Chats, die sehr lang werden".
    let starts = |w: &str, stems: &[&str]| stems.iter().any(|s| w.starts_with(s));
    let chat_word = |w: &str| CHAT_STEMS.iter().any(|s| if *s == "rp" || *s == "role" { w == *s } else { w.starts_with(s) });
    for (i, (start, w)) in ws.iter().enumerate() {
        if !starts(w, &LONG_STEMS) || w.starts_with("langsam") || w.starts_with("langweil") {
            continue;
        }
        let after = ws.iter().skip(i + 1).take(3).position(|x| chat_word(x.1)).map(|k| i + 1 + k);
        let before = (i.saturating_sub(4)..i).rev().find(|j| chat_word(ws[*j].1));
        // "lange Antworten" is about the length of a reply, not of the chat.
        let reply_next = ws.get(i + 1).is_some_and(|x| x.1.starts_with("antwort") || x.1.starts_with("repl") || x.1.starts_with("answer") || x.1.starts_with("response"));
        if reply_next {
            continue;
        }
        if let Some(j) = after {
            let end = ws[j].0 + ws[j].1.len();
            if !negated(&ws, i, &norm, end) {
                add(Need::LongChats, &norm[*start..end]);
            }
        } else if let Some(j) = before {
            let end = start + w.len();
            if !negated(&ws, i, &norm, end) {
                add(Need::LongChats, &norm[ws[j].0..end]);
            }
        }
    }

    // "3–4 User-Charaktere", "drei Figuren": the largest number counts.
    let mut figures: Option<u8> = None;
    for (i, (start, w)) in ws.iter().enumerate() {
        let n = w.parse::<u8>().ok().or_else(|| NUMBER_WORDS.iter().find(|(name, _)| name == w).map(|(_, n)| *n));
        let Some(n) = n.filter(|n| (2..=12).contains(n)) else { continue };
        let Some(j) = ws.iter().skip(i + 1).take(3).position(|x| starts(x.1, &FIGURE_STEMS)).map(|k| i + 1 + k) else { continue };
        if negated(&ws, i, &norm, ws[j].0 + ws[j].1.len()) {
            continue;
        }
        if figures.is_none_or(|f| n > f) {
            figures = Some(n);
        }
        add(Need::Group, &norm[*start..ws[j].0 + ws[j].1.len()]);
    }

    let needs: Vec<Need> = Need::ALL.into_iter().filter(|n| hits.iter().any(|h| h.need == *n)).collect();
    hits.sort_by_key(|h| h.need);
    Parsed { needs, figures, hits }
}

// ---------------------------------------------------------------- settings for a purpose

/// Which measured setting of a benchmark fits the needs: the largest context that still reads
/// fluently for long chats, groups and much context (the balanced one when it should be fast as
/// well), the fastest for "fast", else the balanced one.
pub fn bench_purpose(needs: &[Need]) -> bench::Purpose {
    let roomy = needs.iter().any(|n| matches!(n, Need::LongChats | Need::Group | Need::Context));
    let fast = needs.iter().any(|n| matches!(n, Need::Fast | Need::WeakPc));
    match (roomy, fast) {
        (true, false) => bench::Purpose::Context,
        (false, true) => bench::Purpose::Speed,
        _ => bench::Purpose::Balanced,
    }
}

/// Why a sampler value is proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// The model's author recommends it (the catalog's `author_sampler`).
    Author,
    LongChats,
    Group,
    Story,
    Fast,
    ShortReplies,
    LongReplies,
}

/// One proposed sampler value with its reason.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Derived {
    /// Field name as in [`Sampler`].
    pub field: &'static str,
    pub value: f64,
    pub why: Why,
}

/// DRY: multiplier 0.8 as its author suggests and model cards use; base 1.75 and "repeats of up
/// to 2 tokens stay free" are llama.cpp's own defaults.
pub const DRY_MULTIPLIER: f64 = 0.8;
pub const DRY_BASE: f64 = 1.75;
pub const DRY_ALLOWED_LENGTH: i64 = 2;
/// With three or more characters the names repeat all the time: one token more stays free.
pub const DRY_ALLOWED_LENGTH_GROUP: i64 = 3;
/// Reply lengths (tokens): a narrated scene, a short line, a quick answer.
pub const LONG_REPLY: i64 = 1200;
pub const SHORT_REPLY: i64 = 300;
pub const FAST_REPLY: i64 = 500;
/// A group reply: this much for two characters and [`GROUP_REPLY_STEP`] more for each further one.
pub const GROUP_REPLY: i64 = 800;
pub const GROUP_REPLY_STEP: i64 = 150;
/// "Fast" keeps this share of a longer reply length.
pub const FAST_SHARE: f64 = 0.6;

/// The sampler for a purpose, on top of the profile's current one (`current`; what no rule
/// touches stays), and the list of what was derived and why:
/// - the author's values of a catalog model (`author`: temperature, top_p, top_k, min_p, rep_pen,
///   presence_penalty, DRY) first,
/// - long chats and groups switch DRY on (repetition is what breaks long chats),
/// - the reply length follows short/long replies, stories, the number of characters and "fast".
pub fn sampler_for(needs: &[Need], figures: Option<u8>, current: &Sampler, author: Option<&Value>) -> (Sampler, Vec<Derived>) {
    let has = |n: Need| needs.contains(&n);
    let mut s = current.clone();
    let mut derived: Vec<Derived> = Vec::new();
    fn put_in(derived: &mut Vec<Derived>, field: &'static str, value: f64, why: Why) {
        derived.retain(|d| d.field != field);
        derived.push(Derived { field, value, why });
    }

    if let Some(a) = author {
        let mut put = |field: &'static str, value: f64, why: Why| put_in(&mut derived, field, value, why);
        let num = |k: &str| a.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
        if let Some(v) = num("temperature") {
            s.temperature = Some(v);
            put("temperature", v, Why::Author);
        }
        if let Some(v) = num("top_p") {
            s.top_p = Some(v);
            put("top_p", v, Why::Author);
        }
        if let Some(v) = num("top_k") {
            s.top_k = Some(v as i64);
            put("top_k", v, Why::Author);
        }
        if let Some(v) = num("min_p") {
            s.min_p = Some(v);
            put("min_p", v, Why::Author);
        }
        if let Some(v) = num("rep_pen") {
            s.repeat_penalty = Some(v);
            put("repeat_penalty", v, Why::Author);
        }
        if let Some(v) = num("presence_penalty") {
            s.presence_penalty = Some(v);
            put("presence_penalty", v, Why::Author);
        }
        if let Some(v) = num("dry_multiplier") {
            s.dry_multiplier = Some(v);
            put("dry_multiplier", v, Why::Author);
        }
        if let Some(v) = num("dry_base") {
            s.dry_base = Some(v);
            put("dry_base", v, Why::Author);
        }
        if let Some(v) = num("dry_allowed_length") {
            s.dry_allowed_length = Some(v as i64);
            put("dry_allowed_length", v, Why::Author);
        }
    }

    if has(Need::LongChats) || has(Need::Group) {
        // DRY is on for this purpose. A value the profile or the author already set stays; the
        // list names the value in effect either way (an author's value keeps its own reason), so
        // it reads the same before and after "Setzen".
        let why = if has(Need::LongChats) { Why::LongChats } else { Why::Group };
        let by_author = |derived: &[Derived], field: &str| derived.iter().any(|d| d.field == field && d.why == Why::Author);
        let multiplier = s.dry_multiplier.filter(|m| *m > 0.0).unwrap_or(DRY_MULTIPLIER);
        let base = s.dry_base.unwrap_or(DRY_BASE);
        let wanted = if has(Need::Group) && figures.is_some_and(|f| f >= 3) { DRY_ALLOWED_LENGTH_GROUP } else { DRY_ALLOWED_LENGTH };
        let group_rule = wanted == DRY_ALLOWED_LENGTH_GROUP;
        let allowed = match s.dry_allowed_length {
            Some(l) if !group_rule || l >= wanted => l,
            _ => wanted,
        };
        (s.dry_multiplier, s.dry_base, s.dry_allowed_length) = (Some(multiplier), Some(base), Some(allowed));
        for (field, value, why) in [("dry_multiplier", multiplier, why), ("dry_base", base, why), ("dry_allowed_length", allowed as f64, if group_rule { Why::Group } else { why })] {
            if !by_author(&derived, field) || (field == "dry_allowed_length" && group_rule && derived.iter().any(|d| d.field == field && d.value != value)) {
                derived.retain(|d| d.field != field);
                derived.push(Derived { field, value, why });
            }
        }
    }

    let length: Option<(i64, Why)> = if has(Need::ShortReplies) {
        Some((SHORT_REPLY, Why::ShortReplies))
    } else {
        let base = if has(Need::LongReplies) {
            Some((LONG_REPLY, Why::LongReplies))
        } else if has(Need::Story) {
            Some((LONG_REPLY, Why::Story))
        } else if has(Need::Group) {
            let n = i64::from(figures.unwrap_or(3).clamp(2, 6));
            Some((GROUP_REPLY + GROUP_REPLY_STEP * (n - 2), Why::Group))
        } else {
            None
        };
        match (base, has(Need::Fast)) {
            (Some((n, _)), true) => Some((((n as f64 * FAST_SHARE / 50.0).round() as i64) * 50, Why::Fast)),
            (None, true) => Some((FAST_REPLY, Why::Fast)),
            (base, false) => base,
        }
    };
    if let Some((n, why)) = length {
        s.n_predict = Some(n);
        put_in(&mut derived, "n_predict", n as f64, why);
    }
    (s, derived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use Need::*;

    fn needs(text: &str) -> Vec<Need> {
        parse(text).needs
    }

    /// German and English sentences, among them the owner's example.
    #[test]
    fn sentences() {
        let cases: &[(&str, &[Need], Option<u8>)] = &[
            ("Ich will richtig lange Chats mit 3–4 User-Charakteren", &[LongChats, Group], Some(4)),
            ("Schnelle Antworten auf Deutsch, mein PC ist nicht der neueste", &[German, Fast], None),
            ("Ich möchte ein Fantasy-Abenteuer erzählt bekommen, gern ausführlich", &[Story, LongReplies], None),
            ("unzensiertes Rollenspiel auf Deutsch", &[German, Uncensored], None),
            ("Gruppenchat mit vier Figuren, die sich nicht vermischen", &[Group], Some(4)),
            ("Ich habe nur einen alten Laptop ohne Grafikkarte", &[WeakPc], None),
            ("Viel Kontext für mein Lorebook, es soll nichts vergessen", &[Context], None),
            ("I want long roleplay sessions that go on for hours", &[LongChats], None),
            ("fast replies, uncensored, English only", &[English, Fast, Uncensored], None),
            ("A text adventure with a party of 5 characters", &[Group, Story], Some(5)),
            ("best quality for storytelling, speed doesn't matter", &[Story, Quality], None),
            ("something small for a weak PC, cpu only", &[WeakPc], None),
            ("Kurze Antworten wie im Chat, kein Roman", &[ShortReplies], None),
            ("Ein Modell mit gutem Gedächtnis für hunderte Nachrichten", &[LongChats, Context], None),
            ("Tempo ist egal, Hauptsache die beste Qualität auf Deutsch", &[German, Quality], None),
            ("multiple characters in a long campaign, german", &[LongChats, Group, German], None),
            ("Ich will fast nur Deutsch schreiben", &[German], None),
            ("Erotik ohne Zensur, lange Antworten", &[Uncensored, LongReplies], None),
            ("nicht auf Deutsch, lieber Englisch und schnell", &[English, Fast], None),
            ("128k Kontext und D&D mit zwei Spielern", &[Group, Story, Context], Some(2)),
            ("", &[], None),
            ("Hallo, was kannst du?", &[], None),
        ];
        for (text, want, figures) in cases {
            let p = parse(text);
            assert_eq!(&p.needs, want, "{text}: {:?}", p.hits);
            assert_eq!(p.figures, *figures, "{text}");
            assert_eq!(p.hits.len(), p.needs.len(), "one reason per need: {text}");
        }
    }

    #[test]
    fn hits_name_the_words() {
        let p = parse("Ich will richtig lange Chats mit 3–4 User-Charakteren");
        assert_eq!(p.hits, vec![Hit { need: LongChats, phrase: "lange chats".into() }, Hit { need: Group, phrase: "3 4 user charakteren".into() }]);
        assert_eq!(parse("SCHNELLE Antworten!").hits[0].phrase, "schnelle");
        // "langsam" and "langweilig" are not "lang".
        assert_eq!(needs("Der Chat ist mir zu langsam"), vec![]);
        assert_eq!(needs("lange Antworten im Chat"), vec![LongReplies]);
    }

    #[test]
    fn a_purpose_keeps_corrected_needs_in_chip_order() {
        let p = Purpose::with_needs("lange Chats mit 3 Figuren", &[Fast, LongChats, Fast]);
        assert_eq!((p.needs.clone(), p.figures), (vec![LongChats, Fast], Some(3)));
        let stored = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Purpose>(&stored).unwrap(), p);
        assert_eq!(serde_json::from_str::<Purpose>("{}").unwrap(), Purpose::default());
        assert_eq!(serde_json::to_value(Need::WeakPc).unwrap(), "weak_pc");
    }

    #[test]
    fn the_measured_setting_follows_the_needs() {
        assert_eq!(bench_purpose(&[LongChats, Group]), bench::Purpose::Context);
        assert_eq!(bench_purpose(&[Fast]), bench::Purpose::Speed);
        assert_eq!(bench_purpose(&[LongChats, Fast]), bench::Purpose::Balanced);
        assert_eq!(bench_purpose(&[German]), bench::Purpose::Balanced);
        assert_eq!(bench_purpose(&[]), bench::Purpose::Balanced);
    }

    /// The owner's example: long chats with 3–4 characters switch DRY on and give a reply room for
    /// four characters; what the profile already sets stays.
    #[test]
    fn sampler_for_long_group_chats() {
        let p = Purpose::from_text("Ich will richtig lange Chats mit 3–4 User-Charakteren");
        let current = Sampler { temperature: Some(0.9), seed: Some(7), ..Sampler::default() };
        let (s, derived) = sampler_for(&p.needs, p.figures, &current, None);
        assert_eq!((s.dry_multiplier, s.dry_base, s.dry_allowed_length), (Some(0.8), Some(1.75), Some(3)));
        assert_eq!(s.n_predict, Some(1100), "800 for two characters + 150 for each of the other two");
        assert_eq!((s.temperature, s.seed), (Some(0.9), Some(7)), "untouched values stay");
        assert_eq!(s.check(), Ok(()));
        let why = |f: &str| derived.iter().find(|d| d.field == f).map(|d| d.why);
        assert_eq!((why("dry_multiplier"), why("dry_allowed_length"), why("n_predict"), why("temperature")), (Some(Why::LongChats), Some(Why::Group), Some(Why::Group), None));
    }

    #[test]
    fn sampler_rules() {
        let none = Sampler::default();
        // The author's values come first; DRY from the card is not overwritten by the rule.
        let author = json!({ "temperature": 1.0, "top_p": null, "top_k": 64, "min_p": 0.1, "rep_pen": 1.05, "dry_multiplier": 0.6, "source": "card" });
        let (s, derived) = sampler_for(&[LongChats], None, &none, Some(&author));
        assert_eq!((s.temperature, s.top_p, s.top_k, s.min_p, s.repeat_penalty), (Some(1.0), None, Some(64), Some(0.1), Some(1.05)));
        assert_eq!((s.dry_multiplier, s.dry_base, s.dry_allowed_length), (Some(0.6), Some(1.75), Some(2)));
        assert_eq!(derived.iter().find(|d| d.field == "dry_multiplier").map(|d| d.why), Some(Why::Author));
        assert_eq!(s.n_predict, None, "no rule for the length: the profile's stays");
        // Lengths.
        let len = |needs: &[Need], figures| sampler_for(needs, figures, &none, None).0.n_predict;
        assert_eq!(len(&[Story], None), Some(1200));
        assert_eq!(len(&[Story, Fast], None), Some(700));
        assert_eq!(len(&[Fast], None), Some(500));
        assert_eq!(len(&[ShortReplies, Story, Fast], None), Some(300));
        assert_eq!(len(&[Group], None), Some(950));
        assert_eq!(len(&[Group], Some(2)), Some(800));
        assert_eq!(len(&[Group], Some(12)), Some(1400));
        assert_eq!(len(&[German, Uncensored], None), None);
        // No need, no author: nothing changes.
        assert_eq!(sampler_for(&[], None, &Sampler::legacy(), None), (Sampler::legacy(), vec![]));
        // A profile that already has DRY keeps its values; the list names them as they are.
        let own = Sampler { dry_multiplier: Some(1.0), dry_base: Some(2.0), dry_allowed_length: Some(4), ..Sampler::default() };
        let (kept, listed) = sampler_for(&[LongChats, Group], Some(4), &own, None);
        assert_eq!(kept, Sampler { n_predict: Some(1100), ..own.clone() });
        assert_eq!(listed.iter().map(|d| (d.field, d.value)).collect::<Vec<_>>(), [("dry_multiplier", 1.0), ("dry_base", 2.0), ("dry_allowed_length", 4.0), ("n_predict", 1100.0)]);
        // The advice is stable: applied once, the same needs propose the same sampler and the same list.
        let (first, list) = sampler_for(&[LongChats, Group], Some(4), &none, None);
        assert_eq!(sampler_for(&[LongChats, Group], Some(4), &first, None), (first, list));
    }
}
