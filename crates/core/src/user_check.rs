//! What a card says about the user's figure (`{{user}}`), compared with a persona. A local text
//! heuristic without a model: it finds the places of the card that are
//! about `{{user}}` and sorts each into kinds.
//!
//! Kinds: `role` (relationship, job, situation; the default), `appearance`, `personality`,
//! `gender` (only from a noun or word bound to `{{user}}`: "{{user}} ist ihr Stiefsohn",
//! "her husband {{user}}", "Gender: female") and `name` ("{{user}} heißt Kai", "named Kai",
//! "Name: Kai" in a `[{{user}}]` section).
//!
//! Against a persona: `conflict` = the card's gender does not match the persona's pronouns
//! (er/ihm ↔ male, sie/ihr ↔ female; other or empty pronouns never conflict) or the card names
//! `{{user}}` differently; `overlap` = the card describes appearance / personality and the persona
//! has that field filled. Only gender, name, appearance and personality affect a non-OK result.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::card::Card;
use crate::prompt::UserFigure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    Ok,
    Overlap,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Role,
    Appearance,
    Personality,
    Gender,
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gender {
    Male,
    Female,
}

impl Gender {
    fn as_str(self) -> &'static str {
        match self {
            Gender::Male => "male",
            Gender::Female => "female",
        }
    }
}

/// One place of the card about `{{user}}`, for one kind.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub kind: Kind,
    /// The card field: description | personality | scenario | system_prompt | first_mes.
    pub field: &'static str,
    /// The sentence (≤ 200 characters), macros as written (`<user>` shows as `{{user}}`).
    pub excerpt: String,
    pub verdict: Fit,
    /// What the card sets: "male" / "female" for gender, the name for name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The persona's side of an overlap or conflict (its pronouns, name or field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The worst verdict of all items (`ok` without items).
    pub status: Fit,
    pub items: Vec<Item>,
}

const EXCERPT_MAX: usize = 200;

/// `{{user}}`, `{{ User }}`, `<user>`, `<USER>`.
static USER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\{\{\s*user\s*\}\}|<user>").unwrap());
static CHAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\{\{\s*char\s*\}\}|<bot>").unwrap());
const U: &str = "{{user}}";
const C: &str = "{{char}}";

/// A section heading on its own line: `[Name]` or `# Name`.
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:\[([^\]\n]{1,60})\]|#{1,6}\s+(.{1,60}?))\s*:?\s*$").unwrap());

/// Predicate after a verb: "{{user}} ist ein Mann", "{{user}} is her husband".
static COPULA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\{\{user\}\}\s+(?:ist|bist|war|wird|wurde|spielt|is|was|plays|becomes|als|as)\s+([^,;:.!?()\[\]\x22„“\n]*)").unwrap()
});

/// Apposition set off on both sides: "{{user}}, ihr Stiefsohn, …", "{{user}} (male)".
static APPOSITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{user\}\}\s*(?:,\s*([^,;:.!?()\n]*),|\(\s*([^()\n]*)\))").unwrap());

/// Noun right before `{{user}}` after a possessive: "ihr Sohn {{user}}", "her beloved son {{user}}".
static BEFORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?:ihr|ihre|ihren|ihrem|ihres|sein|seine|seinen|seinem|seines|dein|deine|deinen|mein|meine|meinen|unser|unsere",
        r"|her|his|my|your|their|our)\s+(?:[\p{L}-]+\s+)?([\p{L}é-]+)\s+\{\{user\}\}",
    ))
    .unwrap()
});

/// Pronouns right after the name: "{{user}} (sie/ihr)".
static PRONOUN_AFTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\{\{user\}\}\s*\(\s*(er|sie|he|she)\s*/").unwrap());

/// "Geschlecht: weiblich", W++ `Gender("male")`.
static GENDER_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\b(?:geschlecht|gender|sex)\s*[:=(]\s*["'„“]?\s*(\p{L}+)"#).unwrap());

const NAME: &str = r"(\p{Lu}[\p{L}'-]*(?:\s+\p{Lu}[\p{L}'-]*)?)";

/// Names the card gives `{{user}}`.
static NAME_SET: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        format!(r#"\{{\{{user\}}\}}\s+(?:(?:is|was|wird|wurde)\s+)?(?:heißt|heisst|hieß|nennt sich|namens|named|called|goes by(?: the name)?)\s+["'„“]?{NAME}"#),
        format!(r#"\{{\{{user\}}\}}(?:s|'s)\s+(?:Name|name)\s+(?:ist|is|lautet)\s+["'„“]?{NAME}"#),
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});

/// "Name: Kai" / W++ `Name("Kai")` inside what is about `{{user}}`.
static NAME_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r#"(?i:\bname)\s*[:=(]\s*["'„“]?\s*{NAME}"#)).unwrap());

static APPEARANCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?:aussehen|aussehend\w*|gutaussehend\w*|haar\w*|\w+haarig\w*|augen|größe|statur|körper\w*|muskulös\w*|schlank\w*|kräftig\w*",
        r"|bart\w*|narbe\w*|tattoo\w*|tätowier\w*|haut|gesicht\w*|attraktiv\w*|hübsch\w*|brille|sommersprossen|blond\w*|brünett\w*",
        r"|appearance|looks like|hair\w*|eyes|height|tall|build|muscular|slim|slender|athletic|beard|scars?|tattoos?|skin|face",
        r"|handsome|pretty|attractive|glasses|freckles|brunette|redhead|wears|wearing|dressed)\b",
        r"|\b\d{3}\s*cm\b",
    ))
    .unwrap()
});

static PERSONALITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(?:persönlichkeit|charakter|wesen|schüchtern\w*|selbstbewusst\w*|freundlich\w*|mutig\w*|ruhig\w*|frech\w*|nett\w*",
        r"|arrogant\w*|introvertiert\w*|extrovertiert\w*|humorvoll\w*|sarkastisch\w*|stur\w*|loyal\w*|ehrlich\w*|lustig\w*|gelassen\w*",
        r"|personality|shy|confident|brave|quiet|cocky|introverted|extroverted|funny|sarcastic|stubborn|honest|calm|friendly",
        r"|caring|cheerful|grumpy|lazy|kind-hearted|kindhearted)\b",
    ))
    .unwrap()
});

static ROLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        // No \b in front: German compounds ("Trainingsgefährte"). A role is never a conflict.
        r"(?i)(?:freund\w*|nachbar\w*|student\w*|studier\w*|kolleg\w*|chef\w*|partner\w*|verheiratet|gefährt\w*|mitbewohner\w*",
        r"|wohnt|lebt|arbeitet|beruf|job|lehrer\w*|\w*sohn|\w*tochter|\w*bruder|\w*schwester|\w*mutter|\w*vater|ehe\w*|verlobt\w*",
        r"|meister\w*|diener\w*|kunde\w*|gast|besucher\w*|fremde\w*|reisende\w*|abenteurer\w*|ritter|soldat\w*|kapitän\w*|rolle",
        r"|friend\w*|neighbou?r\w*|students?|colleague\w*|coworker\w*|boss|married|companion\w*|roommate\w*|lives|works|teacher\w*",
        r"|son|daughter|brother|sister|mother|father|husband|wife|master|servant|customer|guest|visitor|stranger|travell?er",
        r"|adventurer|knight|soldier|captain|role)\b",
    ))
    .unwrap()
});

/// Words that end a predicate before its head noun shows up.
const STOP: &[&str] = &[
    "von", "vom", "mit", "bei", "für", "aus", "zu", "zum", "zur", "an", "am", "in", "im", "auf", "über", "und", "oder", "aber",
    "of", "at", "with", "for", "from", "to", "on", "and", "or", "but", "who", "that", "which", "whose",
];
/// A genitive after the head noun: "ein Kollege ihrer Schwester" says nothing about `{{user}}`.
const GENITIVE: &[&str] = &["der", "des", "ihrer", "seiner", "eines", "einer", "deren", "dessen"];

const EN_MALE: &[&str] = &[
    "man", "guy", "boy", "son", "stepson", "brother", "stepbrother", "husband", "father", "stepfather", "dad", "uncle", "nephew",
    "grandson", "boyfriend", "fiancé", "prince", "king", "male", "gentleman", "widower", "groom", "lord", "sir", "dude", "lad",
    "männlich",
];
const EN_FEMALE: &[&str] = &[
    "woman", "girl", "lady", "daughter", "stepdaughter", "sister", "stepsister", "wife", "mother", "stepmother", "mom", "mum",
    "aunt", "niece", "granddaughter", "girlfriend", "fiancée", "princess", "queen", "female", "mistress", "widow", "bride",
    "madam", "housewife", "weiblich",
];
/// German nouns (capitalized), matched at the end of compounds ("Stiefsohn", "Ehefrau").
const DE_MALE: &[&str] = &[
    "mann", "sohn", "bruder", "vater", "onkel", "neffe", "cousin", "schwager", "bräutigam", "witwer", "herr", "könig", "prinz",
    "gatte", "gemahl", "junge", "kerl", "bursche", "verlobter", "freund",
];
const DE_MALE_EXACT: &[&str] = &["opa", "enkel"];
const DE_FEMALE: &[&str] = &[
    "frau", "tochter", "schwester", "mutter", "tante", "nichte", "cousine", "braut", "witwe", "dame", "mädchen", "herrin",
    "königin", "prinzessin", "gattin", "gemahlin", "freundin", "göttin", "ärztin", "chefin", "kundin", "kollegin", "verlobte",
];
const DE_FEMALE_EXACT: &[&str] = &["oma"];
/// Female forms of German agent nouns: "Nachbarin", "Studentin", "Lehrerin".
const DE_FEMALE_SUFFIX: &[&str] = &["erin", "entin", "antin", "istin", "arin", "orin", "eurin", "login", "atin"];

fn word_gender(word: &str) -> Option<Gender> {
    let lower = word.to_lowercase();
    if EN_FEMALE.contains(&lower.as_str()) {
        return Some(Gender::Female);
    }
    if EN_MALE.contains(&lower.as_str()) {
        return Some(Gender::Male);
    }
    if !word.chars().next().is_some_and(char::is_uppercase) {
        return None; // German nouns are capitalized: "eine junge Frau" is not about a "Junge"
    }
    if DE_FEMALE_EXACT.contains(&lower.as_str())
        || DE_FEMALE.iter().any(|s| lower.ends_with(s))
        || (lower.chars().count() >= 6 && DE_FEMALE_SUFFIX.iter().any(|s| lower.ends_with(s)))
    {
        return Some(Gender::Female);
    }
    if DE_MALE_EXACT.contains(&lower.as_str()) || DE_MALE.iter().any(|s| lower.ends_with(s)) {
        return Some(Gender::Male);
    }
    None
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphabetic() || c == 'é')).filter(|w| !w.is_empty())
}

/// The gender of a predicate's head noun: the first gendered word, unless a preposition,
/// conjunction or a genitive after a noun comes first.
fn predicate_gender(text: &str) -> Option<Gender> {
    let mut seen_noun = false;
    for word in words(text).take(8) {
        if let Some(g) = word_gender(word) {
            return Some(g);
        }
        let lower = word.to_lowercase();
        if STOP.contains(&lower.as_str()) || (seen_noun && GENITIVE.contains(&lower.as_str())) {
            return None;
        }
        seen_noun |= word.chars().next().is_some_and(char::is_uppercase);
    }
    None
}

fn key_gender(word: &str) -> Option<Gender> {
    match word.to_lowercase().as_str() {
        "m" | "male" | "man" | "männlich" | "mann" => Some(Gender::Male),
        "w" | "f" | "female" | "woman" | "weiblich" | "frau" => Some(Gender::Female),
        _ => None,
    }
}

/// The gender a sentence gives `{{user}}`. `region` is the part about `{{user}}`.
fn gender(sentence: &str, region: &str, in_section: bool) -> Option<Gender> {
    if let Some(c) = PRONOUN_AFTER.captures(sentence) {
        return Some(if matches!(c[1].to_lowercase().as_str(), "er" | "he") { Gender::Male } else { Gender::Female });
    }
    if let Some(g) = GENDER_KEY.captures(region).and_then(|c| key_gender(&c[1])) {
        return Some(g);
    }
    if let Some(g) = COPULA.captures_iter(sentence).find_map(|c| predicate_gender(&c[1])) {
        return Some(g);
    }
    for c in APPOSITION.captures_iter(sentence) {
        let inner = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
        if words(inner).count() <= 5 {
            if let Some(g) = predicate_gender(inner) {
                return Some(g);
            }
        }
    }
    for c in BEFORE.captures_iter(sentence) {
        // "die Mutter {{user}}s" is the mother of {{user}}, not {{user}}.
        let after = &sentence[c.get(0).unwrap().end()..];
        if !(after.starts_with('s') || after.starts_with('\'') || after.starts_with('’')) {
            if let Some(g) = word_gender(&c[1]) {
                return Some(g);
            }
        }
    }
    if in_section && !sentence.contains(U) {
        // "[{{user}}]\nEin junger Mann, …"
        let first = words(sentence).next().unwrap_or("").to_lowercase();
        if matches!(first.as_str(), "ein" | "eine" | "a" | "an") {
            let head = sentence.split([',', ';', ':', '(']).next().unwrap_or("");
            return predicate_gender(head);
        }
    }
    None
}

/// A name the card gives `{{user}}`.
fn name(sentence: &str, region: &str) -> Option<String> {
    let found = NAME_SET.iter().find_map(|re| re.captures(sentence)).or_else(|| NAME_KEY.captures(region))?;
    let name = found[1].trim().to_string();
    let lower = name.to_lowercase();
    (!["der", "die", "das", "the", "sie", "er", "ihn", "ihm", "user", "nutzer"].contains(&lower.as_str())).then_some(name)
}

fn same_name(card: &str, persona: &str) -> bool {
    let (c, p) = (card.trim().to_lowercase(), persona.trim().to_lowercase());
    let first = |s: &str| s.split_whitespace().next().unwrap_or("").to_string();
    c == p || p.split_whitespace().any(|w| w == first(&c)) || c.split_whitespace().any(|w| w == first(&p))
}

/// The gender a persona's pronouns stand for; None for other or mixed pronouns.
pub fn pronoun_gender(pronouns: &str) -> Option<Gender> {
    let mut male = false;
    let mut female = false;
    for w in words(&pronouns.to_lowercase()) {
        male |= matches!(w, "er" | "ihm" | "ihn" | "sein" | "he" | "him" | "his");
        female |= matches!(w, "sie" | "ihr" | "she" | "her" | "hers");
    }
    match (male, female) {
        (true, false) => Some(Gender::Male),
        (false, true) => Some(Gender::Female),
        _ => None,
    }
}

fn excerpt(sentence: &str) -> String {
    let s = sentence.trim();
    if s.chars().count() <= EXCERPT_MAX {
        return s.to_string();
    }
    let mut out: String = s.chars().take(EXCERPT_MAX - 1).collect();
    out.push('…');
    out
}

/// Splits a line into sentences after `.`, `!`, `?` or `…` followed by a space.
fn sentences(line: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut start = 0;
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?' | '…') && chars.peek().is_some_and(|(_, n)| n.is_whitespace()) {
            let end = i + c.len_utf8();
            out.push(&line[start..end]);
            start = end;
        }
    }
    out.push(&line[start..]);
    out.into_iter().filter(|s| !s.trim().is_empty()).collect()
}

/// Whether a heading names a section about `{{user}}`: "[{{user}}]", "## {{user}}'s Info".
fn user_heading(inner: &str) -> bool {
    if !inner.contains(U) || inner.contains(C) {
        return false;
    }
    let rest = inner.replace(U, "");
    let rest = rest.trim_matches(|c: char| !c.is_alphanumeric());
    rest.chars().count() <= 24 && !rest.contains([':', '(', ';', '=', '"', ','])
}

/// The places of one field about `{{user}}`: (sentence, inside a `[{{user}}]` section).
fn mentions(text: &str) -> Vec<(String, bool)> {
    let text = USER.replace_all(text, U);
    let text = CHAR.replace_all(&text, C);
    let mut out = vec![];
    let mut in_section = false;
    for line in text.lines() {
        if let Some(c) = HEADING.captures(line) {
            let inner = c.get(1).or(c.get(2)).map_or("", |m| m.as_str());
            // A bracket line about {{user}} with content (W++ "[{{user}}: ...]") is text, not a heading.
            if !inner.contains(U) || user_heading(inner) {
                in_section = user_heading(inner);
                continue;
            }
        }
        for s in sentences(line) {
            if in_section || s.contains(U) {
                out.push((s.trim().to_string(), in_section));
            }
        }
    }
    out
}

/// The part of a sentence about `{{user}}`: from its first mention up to `{{char}}` / the
/// character's name. A sentence in a `[{{user}}]` section without the macro is all about them.
fn region<'a>(sentence: &'a str, char_name: &str) -> &'a str {
    let Some(start) = sentence.find(U) else { return sentence };
    let rest = &sentence[start..];
    let mut end = rest.find(C).unwrap_or(rest.len());
    let name = char_name.split_whitespace().next().unwrap_or("");
    if name.chars().count() >= 2 {
        if let Some(i) = rest.find(name) {
            end = end.min(i);
        }
    }
    &rest[..end]
}

/// Checks the card against the persona `user_name` with these fields.
pub fn check(card: &Card, user_name: &str, persona: &UserFigure) -> Report {
    let persona_gender = pronoun_gender(&persona.pronouns);
    let filled = |t: &str| (!t.trim().is_empty()).then(|| excerpt(t));
    let fields = [
        ("description", &card.description),
        ("personality", &card.personality),
        ("scenario", &card.scenario),
        ("system_prompt", &card.system_prompt),
        ("first_mes", &card.first_mes),
    ];
    let mut items: Vec<Item> = vec![];
    for (field, text) in fields {
        for (sentence, in_section) in mentions(text) {
            let region = region(&sentence, &card.name);
            let mut found: Vec<(Kind, Fit, Option<String>, Option<String>)> = vec![];
            if let Some(g) = gender(&sentence, region, in_section) {
                let fit = if persona_gender.is_some_and(|p| p != g) { Fit::Conflict } else { Fit::Ok };
                let theirs = (fit == Fit::Conflict).then(|| persona.pronouns.trim().to_string());
                found.push((Kind::Gender, fit, Some(g.as_str().into()), theirs));
            }
            if let Some(n) = name(&sentence, region) {
                let fit = if same_name(&n, user_name) { Fit::Ok } else { Fit::Conflict };
                let theirs = (fit == Fit::Conflict).then(|| user_name.to_string());
                found.push((Kind::Name, fit, Some(n), theirs));
            }
            for (kind, re, own) in [(Kind::Appearance, &*APPEARANCE, &persona.appearance), (Kind::Personality, &*PERSONALITY, &persona.personality)] {
                if re.is_match(region) {
                    let theirs = filled(own);
                    found.push((kind, if theirs.is_some() { Fit::Overlap } else { Fit::Ok }, None, theirs));
                }
            }
            if found.is_empty() || ROLE.is_match(region) {
                found.push((Kind::Role, Fit::Ok, None, None));
            }
            let excerpt = excerpt(&sentence);
            for (kind, verdict, value, persona_text) in found {
                if !items.iter().any(|i| i.kind == kind && i.field == field && i.excerpt == excerpt) {
                    items.push(Item { kind, field, excerpt: excerpt.clone(), verdict, value, persona_text });
                }
            }
        }
    }
    Report { status: items.iter().map(|i| i.verdict).max().unwrap_or(Fit::Ok), items }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(description: &str) -> Card {
        Card { name: "Eris".into(), description: description.into(), ..Card::default() }
    }

    fn persona(pronouns: &str, appearance: &str, personality: &str) -> UserFigure {
        UserFigure { pronouns: pronouns.into(), appearance: appearance.into(), personality: personality.into(), ..UserFigure::default() }
    }

    fn kinds(r: &Report) -> Vec<(Kind, Fit)> {
        r.items.iter().map(|i| (i.kind, i.verdict)).collect()
    }

    fn of(r: &Report, kind: Kind) -> Vec<&Item> {
        r.items.iter().filter(|i| i.kind == kind).collect()
    }

    #[test]
    fn role_only_is_ok_de_and_en() {
        let p = persona("sie/ihr", "Groß, rote Haare.", "Ruhig.");
        let r = check(&card("{{user}}, 20, im zweiten Studienjahr, wohnt zwei Türen weiter."), "Lena", &p);
        assert_eq!((r.status, kinds(&r)), (Fit::Ok, vec![(Kind::Role, Fit::Ok)]));
        let r = check(&card("Eris travels the north. {{user}} is her travel companion."), "Lena", &p);
        assert_eq!(r.status, Fit::Ok);
        assert_eq!(r.items.len(), 1, "only the sentence about {{user}}: {:?}", r.items);
        assert_eq!(r.items[0].excerpt, "{{user}} is her travel companion.");
        let r = check(&Card { name: "Eris".into(), system_prompt: "{{user}} spielt nur sich selbst.".into(), ..Card::default() }, "Lena", &p);
        assert_eq!((r.items[0].field, r.items[0].kind), ("system_prompt", Kind::Role));
    }

    #[test]
    fn appearance_overlaps_a_filled_persona_field() {
        let c = card("{{user}} hat kurze schwarze Haare und eine Narbe am Kinn.");
        let r = check(&c, "Lena", &persona("", "Lange blonde Haare.", ""));
        assert_eq!(r.status, Fit::Overlap);
        let a = of(&r, Kind::Appearance);
        assert_eq!((a[0].verdict, a[0].persona_text.as_deref()), (Fit::Overlap, Some("Lange blonde Haare.")));
        // Persona without appearance: nothing to clash with.
        assert_eq!(check(&c, "Lena", &persona("", " ", "")).status, Fit::Ok);
    }

    #[test]
    fn personality_overlap_en() {
        let r = check(&card("{{user}} is shy and quiet around strangers."), "Sam", &persona("he/him", "", "Loud and cocky."));
        assert_eq!(of(&r, Kind::Personality)[0].verdict, Fit::Overlap);
        assert_eq!(r.status, Fit::Overlap);
    }

    #[test]
    fn man_against_she_her_is_a_conflict() {
        let c = card("{{user}} ist ein Mann, ihr Reise- und Trainingsgefährte.");
        let r = check(&c, "Lena", &persona("sie/ihr", "", ""));
        assert_eq!(r.status, Fit::Conflict);
        let g = of(&r, Kind::Gender);
        assert_eq!((g[0].value.as_deref(), g[0].persona_text.as_deref()), (Some("male"), Some("sie/ihr")));
        assert!(of(&r, Kind::Role).len() == 1, "the companion part is a role too");
        assert_eq!(check(&c, "Lukas", &persona("er/ihm", "", "")).status, Fit::Ok);
        assert_eq!(check(&c, "Kim", &persona("they/them", "", "")).status, Fit::Ok, "other pronouns never conflict");
        assert_eq!(check(&c, "Kim", &persona("", "", "")).status, Fit::Ok, "empty pronouns never conflict");
    }

    #[test]
    fn kin_nouns_set_the_gender() {
        let she = persona("sie/ihr", "", "");
        let he = persona("he/him", "", "");
        let r = check(&card("{{user}} ist Eris Stiefsohn."), "Lena", &she);
        assert_eq!((r.status, of(&r, Kind::Gender)[0].value.as_deref()), (Fit::Conflict, Some("male")));
        assert_eq!(check(&card("{{user}} is her husband of ten years."), "Lena", &persona("she/her", "", "")).status, Fit::Conflict);
        let r = check(&card("Eris adores her daughter {{user}}."), "Sam", &he);
        assert_eq!(of(&r, Kind::Gender)[0].value.as_deref(), Some("female"));
        assert_eq!(check(&card("{{user}}, ihre Nachbarin, kommt oft vorbei."), "Sam", &he).status, Fit::Conflict);
        assert_eq!(check(&card("<USER> is a tall woman with red hair."), "Sam", &he).status, Fit::Conflict);
        assert_eq!(of(&check(&card("{{User}} (sie/ihr) kennt Eris."), "Sam", &he), Kind::Gender)[0].value.as_deref(), Some("female"));
    }

    #[test]
    fn other_name_is_a_conflict() {
        let c = card("{{user}} heißt Kai und arbeitet in der Schmiede.");
        let r = check(&c, "Lena", &persona("", "", ""));
        let n = of(&r, Kind::Name);
        assert_eq!((n[0].verdict, n[0].value.as_deref(), n[0].persona_text.as_deref()), (Fit::Conflict, Some("Kai"), Some("Lena")));
        assert_eq!(check(&c, "Kai", &persona("", "", "")).status, Fit::Ok, "same name");
        assert_eq!(check(&c, "Kai Berger", &persona("", "", "")).status, Fit::Ok, "first name matches");
        let r = check(&card("Long ago {{user}} was named Alex by the monks."), "Sam", &persona("", "", ""));
        assert_eq!((r.status, of(&r, Kind::Name)[0].value.as_deref()), (Fit::Conflict, Some("Alex")));
    }

    #[test]
    fn names_of_other_figures_are_not_the_users() {
        let p = persona("er/ihm", "", "");
        let r = check(&card("Im Original heißt der Sohn Rudeus. {{user}} übernimmt seine Rolle."), "Lukas", &p);
        assert!(of(&r, Kind::Name).is_empty(), "{:?}", r.items);
        assert_eq!(r.status, Fit::Ok);
        let r = check(&card("Nennt {{user}} „{{user}}-kun“."), "Lukas", &p);
        assert!(of(&r, Kind::Name).is_empty());
        let r = check(&card("Her son Tom is named after his father. She calls {{user}} 'sweetie'."), "Lukas", &persona("she/her", "", ""));
        assert_eq!((r.status, r.items.len()), (Fit::Ok, 1), "{:?}", r.items);
    }

    #[test]
    fn side_characters_are_not_the_user() {
        let she = persona("sie/ihr", "", "");
        let r = check(&card("Eris lebt mit ihrem Sohn Tom und ihrem Mann. {{user}} wohnt nebenan."), "Lena", &she);
        assert_eq!((r.status, kinds(&r)), (Fit::Ok, vec![(Kind::Role, Fit::Ok)]));
        let r = check(&card("{{char}} ist {{user}}s Mutter."), "Lukas", &persona("er/ihm", "", ""));
        assert!(of(&r, Kind::Gender).is_empty(), "the mother is the character: {:?}", r.items);
        let r = check(&card("{{user}} ist ein Kollege ihrer Schwester."), "Lukas", &persona("er/ihm", "", ""));
        assert!(of(&r, Kind::Gender).iter().all(|g| g.value.as_deref() == Some("male")), "{:?}", r.items);
        let r = check(&card("{{user}} ist ein Kollege ihrer Schwester."), "Kim", &persona("they/them", "", ""));
        assert_eq!(r.status, Fit::Ok);
        let r = check(&card("Anna mag {{user}}, ihre Schwester aber nicht."), "Lukas", &persona("er/ihm", "", ""));
        assert!(of(&r, Kind::Gender).is_empty(), "{:?}", r.items);
        let r = check(&card("{{char}} has green eyes and knows {{user}} from school."), "Lukas", &persona("", "Tall.", ""));
        assert!(of(&r, Kind::Appearance).is_empty(), "the eyes are the character's: {:?}", r.items);
    }

    #[test]
    fn user_section_until_the_next_heading() {
        let text = "Eris ist Magierin.\n\n[{{user}}]\nGeschlecht: weiblich\nName: Mia\nTrägt eine Brille.\n\n[Welt]\nDer König heißt Arthur. Seine Frau ist Königin.";
        let r = check(&card(text), "Lukas", &persona("er/ihm", "Bart.", ""));
        assert_eq!(r.status, Fit::Conflict);
        assert_eq!(of(&r, Kind::Gender)[0].value.as_deref(), Some("female"));
        assert_eq!(of(&r, Kind::Name)[0].value.as_deref(), Some("Mia"));
        assert_eq!(of(&r, Kind::Appearance)[0].verdict, Fit::Overlap);
        assert!(r.items.iter().all(|i| !i.excerpt.contains("Arthur") && !i.excerpt.contains("Magierin")), "{:?}", r.items);
        let r = check(&card("## {{user}}\nEin junger Mann aus dem Dorf."), "Lena", &persona("sie/ihr", "", ""));
        assert_eq!(r.status, Fit::Conflict);
    }

    #[test]
    fn wpp_line_about_the_user() {
        let r = check(&card("[{{user}}: Gender(\"male\"), Name(\"Ren\"), Hair(\"black\")]"), "Lena", &persona("she/her", "", ""));
        assert_eq!(of(&r, Kind::Gender)[0].verdict, Fit::Conflict);
        assert_eq!(of(&r, Kind::Name)[0].value.as_deref(), Some("Ren"));
        assert_eq!(of(&r, Kind::Appearance)[0].verdict, Fit::Ok);
    }

    #[test]
    fn all_fields_are_read_and_excerpts_stay_short() {
        let long = format!("{{{{user}}}} ist ihr Nachbar und {}", "lang ".repeat(80));
        let c = Card { name: "Eris".into(), scenario: long, first_mes: "*Eris winkt {{user}} zu.*".into(), ..Card::default() };
        let r = check(&c, "Lukas", &persona("er/ihm", "", ""));
        assert_eq!(r.items.iter().map(|i| i.field).collect::<Vec<_>>(), ["scenario", "first_mes"]);
        assert!(r.items.iter().all(|i| i.excerpt.chars().count() <= EXCERPT_MAX));
        assert!(check(&card("Eris lebt allein."), "X", &persona("", "", "")).items.is_empty());
    }

    #[test]
    fn pronouns() {
        assert_eq!(pronoun_gender("er/ihm"), Some(Gender::Male));
        assert_eq!(pronoun_gender("She/Her"), Some(Gender::Female));
        assert_eq!(pronoun_gender("sie/ihr"), Some(Gender::Female));
        assert_eq!(pronoun_gender("er/sie"), None);
        assert_eq!(pronoun_gender("they/them"), None);
        assert_eq!(pronoun_gender(""), None);
    }

    #[test]
    fn serializes_as_the_api_shape() {
        let r = check(&card("{{user}} ist ihr Ehemann."), "Lena", &persona("sie/ihr", "", ""));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["status"], "conflict");
        assert_eq!(v["items"][0]["kind"], "gender");
        assert_eq!(v["items"][0]["verdict"], "conflict");
        assert_eq!(v["items"][0]["field"], "description");
        assert!(v["items"].as_array().unwrap().iter().any(|i| i["kind"] == "role" && i.get("persona_text").is_none()));
    }
}
