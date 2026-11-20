//! The card builder: a new character is planned, filled field by field (by hand or by the local
//! model) and saved as a Character Card V3 (`chara_card_v3`) whose `data` holds every V2 field, so
//! SillyTavern and the app's own import read it the same way.
//!
//! - [`card_json`] turns a [`Draft`] into the card JSON the normal import path stores.
//! - [`draft_from_raw`] reads a stored card back into a draft ("as a template").
//! - [`draft_messages`] is the prompt that asks the local model for one field.
//! - [`clean`] tidies what the model wrote into the value of that field.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::card::{self, Card};
use crate::prompt::{ChatMsg, Language};

/// What the user planned before writing: one sentence of idea and a few anchors.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub idea: String,
    pub genre: String,
    pub tone: String,
    /// Who `{{user}}` is to the figure.
    pub relation: String,
    pub traits: Vec<String>,
    /// What drives the figure and what holds it back (the conflict that carries many scenes).
    pub drive: String,
    /// How the figure speaks.
    pub voice: String,
    pub world: String,
}

/// One lorebook entry: text that goes into the prompt when one of its keys comes up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoreEntry {
    pub name: String,
    pub keys: Vec<String>,
    pub content: String,
    /// Always in the prompt, keys or not.
    pub constant: bool,
    pub enabled: bool,
}

impl Default for LoreEntry {
    fn default() -> Self {
        LoreEntry { name: String::new(), keys: vec![], content: String::new(), constant: false, enabled: true }
    }
}

/// The card as the builder edits it: every field of a V2/V3 card the app knows, plus the
/// lorebook, creator and version.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    pub first_mes: String,
    pub mes_example: String,
    pub system_prompt: String,
    pub post_history_instructions: String,
    pub creator_notes: String,
    pub alternate_greetings: Vec<String>,
    pub tags: Vec<String>,
    pub lore: Vec<LoreEntry>,
    pub creator: String,
    pub version: String,
}

impl Draft {
    /// The card fields the rest of the app works with (trimmed, empty list entries left out).
    pub fn card(&self) -> Card {
        let list = |l: &[String]| l.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        Card {
            name: self.name.trim().to_string(),
            description: self.description.trim().to_string(),
            personality: self.personality.trim().to_string(),
            scenario: self.scenario.trim().to_string(),
            first_mes: self.first_mes.trim().to_string(),
            mes_example: self.mes_example.trim().to_string(),
            system_prompt: self.system_prompt.trim().to_string(),
            post_history_instructions: self.post_history_instructions.trim().to_string(),
            creator_notes: self.creator_notes.trim().to_string(),
            alternate_greetings: list(&self.alternate_greetings),
            tags: list(&self.tags),
        }
    }

    /// The lorebook entries worth keeping: those with text.
    fn kept_lore(&self) -> impl Iterator<Item = &LoreEntry> {
        self.lore.iter().filter(|e| !e.content.trim().is_empty())
    }
}

/// The card JSON of a draft: `chara_card_v3` with every V2 field under `data` (V2 readers take
/// `data` as it is). `now` is the creation time in Unix seconds. The lorebook is only there when
/// an entry has text, so an empty one never makes two equal cards differ.
pub fn card_json(d: &Draft, now: i64) -> Value {
    let c = d.card();
    let mut data = json!({
        "name": c.name,
        "description": c.description,
        "personality": c.personality,
        "scenario": c.scenario,
        "first_mes": c.first_mes,
        "mes_example": c.mes_example,
        "creator_notes": c.creator_notes,
        "system_prompt": c.system_prompt,
        "post_history_instructions": c.post_history_instructions,
        "alternate_greetings": c.alternate_greetings,
        "tags": c.tags,
        "creator": d.creator.trim(),
        "character_version": d.version.trim(),
        "extensions": {},
        "group_only_greetings": [],
        "assets": [{ "type": "icon", "uri": "ccdefault:", "name": "main", "ext": "png" }],
        "creation_date": now,
        "modification_date": now,
    });
    let entries: Vec<Value> = d
        .kept_lore()
        .enumerate()
        .map(|(i, e)| {
            let keys: Vec<String> = e.keys.iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
            json!({
                "id": i,
                "keys": keys,
                "secondary_keys": [],
                "content": e.content.trim(),
                "name": e.name.trim(),
                "comment": e.name.trim(),
                "enabled": e.enabled,
                "constant": e.constant,
                "selective": false,
                "case_sensitive": false,
                "use_regex": false,
                "insertion_order": 100,
                "priority": 10,
                "position": "before_char",
                "extensions": {},
            })
        })
        .collect();
    if !entries.is_empty() {
        data["character_book"] = json!({ "name": c.name, "extensions": {}, "entries": entries });
    }
    json!({ "spec": "chara_card_v3", "spec_version": "3.0", "data": data })
}

/// A stored card as a draft, lorebook included (V1, V2 or V3 JSON). None: not a card.
pub fn draft_from_raw(raw: &Value) -> Option<Draft> {
    let c = card::from_json(raw).ok()?;
    let data = if raw["data"].is_object() { &raw["data"] } else { raw };
    let s = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let lore = data["character_book"]["entries"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    let content = s(&e["content"]);
                    if content.is_empty() {
                        return None;
                    }
                    // V2/V3 name the list `keys`; some older books `key`.
                    let keys = e["keys"].as_array().or_else(|| e["key"].as_array());
                    let keys = keys.map(|k| k.iter().filter_map(|k| k.as_str()).map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect()).unwrap_or_default();
                    let name = Some(s(&e["name"])).filter(|n| !n.is_empty()).unwrap_or_else(|| s(&e["comment"]));
                    Some(LoreEntry { name, keys, content, constant: e["constant"].as_bool().unwrap_or(false), enabled: e["enabled"].as_bool().unwrap_or(true) })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Draft {
        name: c.name,
        description: c.description,
        personality: c.personality,
        scenario: c.scenario,
        first_mes: c.first_mes,
        mes_example: c.mes_example,
        system_prompt: c.system_prompt,
        post_history_instructions: c.post_history_instructions,
        creator_notes: c.creator_notes,
        alternate_greetings: c.alternate_greetings,
        tags: c.tags,
        lore,
        creator: s(&data["creator"]),
        version: s(&data["character_version"]),
    })
}

/// The field the model writes next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Name,
    Description,
    Personality,
    Scenario,
    FirstMes,
    /// One more greeting next to the first message.
    AltGreeting,
    MesExample,
    Tags,
    CreatorNotes,
}

impl Field {
    pub const ALL: [Field; 9] = [
        Field::Name,
        Field::Description,
        Field::Personality,
        Field::Scenario,
        Field::FirstMes,
        Field::AltGreeting,
        Field::MesExample,
        Field::Tags,
        Field::CreatorNotes,
    ];

    /// How many tokens the model may write for it (short fields stay short, even on a CPU).
    pub fn max_tokens(self) -> u32 {
        match self {
            Field::Name => 24,
            Field::Tags => 60,
            Field::Personality => 120,
            Field::CreatorNotes => 160,
            Field::Scenario => 240,
            Field::FirstMes | Field::AltGreeting | Field::MesExample => 520,
            Field::Description => 700,
        }
    }

    /// The order for this field, in German like the app's other prompt rules.
    fn order(self) -> &'static str {
        match self {
            Field::Name => "Schlage einen passenden Namen für die Figur vor. Antworte nur mit dem Namen (Vorname, wenn es passt mit Nachname), ohne weitere Worte.",
            Field::Description => "Schreibe die Beschreibung von {{char}}: Aussehen, Hintergrund, Wesen, was {{char}} antreibt und was {{char}} zurückhält, die Beziehung zu {{user}} und wie {{char}} spricht. 150 bis 300 Wörter in der dritten Person, kurze Absätze.",
            Field::Personality => "Fasse die Persönlichkeit von {{char}} in einer Zeile zusammen: fünf bis zehn Eigenschaften, durch Kommas getrennt.",
            Field::Scenario => "Schreibe das Szenario: wo und wann die Geschichte beginnt und in welcher Lage {{char}} und {{user}} sich begegnen. Zwei bis vier Sätze. Gib keinen Ablauf vor.",
            Field::FirstMes => "Schreibe die erste Nachricht von {{char}}, mit der der Chat beginnt: eine Szene in der dritten Person, wörtliche Rede in Anführungszeichen, Gedanken *kursiv*, 80 bis 200 Wörter. Das Ende bleibt offen, damit {{user}} antworten kann.",
            Field::AltGreeting => "Schreibe eine andere Begrüßung als die erste Nachricht: ein anderer Ort oder eine andere Lage, sonst wie die erste Nachricht (dritte Person, wörtliche Rede in Anführungszeichen, 80 bis 200 Wörter, offenes Ende).",
            Field::MesExample => "Schreibe zwei kurze Beispieldialoge, die zeigen, wie {{char}} spricht. Jeder beginnt mit einer eigenen Zeile <START>, danach abwechselnd Zeilen, die mit „{{user}}:“ und „{{char}}:“ beginnen. Die Zeilen von {{user}} bleiben kurz.",
            Field::Tags => "Nenne drei bis sechs kurze Schlagwörter für die Karte (Genre, Stimmung, Art der Figur), durch Kommas getrennt, ohne weitere Worte.",
            Field::CreatorNotes => "Schreibe eine kurze Notiz für Leser der Karte: worum es geht und was die Figur besonders macht. Ein bis drei Sätze.",
        }
    }
}

/// The builder's own rules, in German like the chat's narration rules.
const GUIDE: &str = "\
Du hilfst beim Schreiben einer Figurenkarte für ein fiktives Rollenspiel. Die Karte beschreibt {{char}}; {{user}} ist die Person, die im Chat mitspielt.

Regeln:
- Schreibe nur den Text des verlangten Feldes: keine Überschrift, keine Erklärung, keine Rückfrage, kein Kommentar.
- Halte dich an den Plan und an die schon geschriebenen Felder; erfinde nichts, was ihnen widerspricht.
- Schreibe nie Handlungen, Gedanken oder wörtliche Rede für {{user}} (außer in Beispieldialogen).
- Gib keine feste Geschichte vor: die Figur braucht einen Antrieb und einen inneren Konflikt, die viele Szenen tragen.
- Schreibe {{char}} und {{user}} genau so, als Platzhalter.";

/// The language the fields are written in: the app's language.
fn language_line(lang: Language) -> &'static str {
    match lang {
        Language::De => "Schreibe auf Deutsch.",
        Language::En => "Schreibe auf Englisch. Write in English.",
        Language::Es => "Schreibe auf Spanisch. Escribe en español.",
        Language::Ru => "Schreibe auf Russisch. Пиши на русском языке.",
    }
}

fn line(out: &mut String, label: &str, value: &str) {
    let value = value.trim();
    if !value.is_empty() {
        out.push_str(label);
        out.push_str(": ");
        out.push_str(value);
        out.push('\n');
    }
}

/// The prompt for one field: the builder's rules, then the plan, the fields written so far and
/// the order. The field itself is left out of "so far", so writing it again gives a new version.
pub fn draft_messages(lang: Language, plan: &Plan, draft: &Draft, field: Field) -> Vec<ChatMsg> {
    let system = format!("{GUIDE}\n- {}", language_line(lang));

    let mut user = String::from("# Plan\n");
    line(&mut user, "Idee", &plan.idea);
    line(&mut user, "Genre", &plan.genre);
    line(&mut user, "Ton", &plan.tone);
    line(&mut user, "Beziehung zu {{user}}", &plan.relation);
    let traits: Vec<&str> = plan.traits.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    line(&mut user, "Kernzüge", &traits.join(", "));
    line(&mut user, "Antrieb und Konflikt", &plan.drive);
    line(&mut user, "Sprechweise", &plan.voice);
    line(&mut user, "Welt", &plan.world);

    let c = draft.card();
    let mut so_far = String::new();
    let skip = |f: Field| f == field;
    if !skip(Field::Name) {
        line(&mut so_far, "Name", &c.name);
    }
    if !skip(Field::Description) {
        line(&mut so_far, "Beschreibung", &c.description);
    }
    if !skip(Field::Personality) {
        line(&mut so_far, "Persönlichkeit", &c.personality);
    }
    if !skip(Field::Scenario) {
        line(&mut so_far, "Szenario", &c.scenario);
    }
    // The greetings stand next to each other: a new one should differ from all of them.
    if !skip(Field::FirstMes) {
        line(&mut so_far, "Erste Nachricht", &c.first_mes);
    }
    for g in &c.alternate_greetings {
        line(&mut so_far, "Weitere Begrüßung", g);
    }
    if !skip(Field::Tags) {
        line(&mut so_far, "Schlagwörter", &c.tags.join(", "));
    }
    if !so_far.is_empty() {
        user.push_str("\n# Die Karte bisher\n");
        user.push_str(&so_far);
    }
    user.push_str("\n# Auftrag\n");
    user.push_str(field.order());
    vec![ChatMsg { role: "system", content: system }, ChatMsg { role: "user", content: user }]
}

/// What the model wrote, as the value of the field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Answer {
    Text(String),
    List(Vec<String>),
}

/// Labels a model likes to put in front of its answer ("Beschreibung: …"); taken off.
const LABELS: [&str; 24] = [
    "name", "vorname", "beschreibung", "description", "persönlichkeit", "personality", "szenario", "scenario", "erste nachricht", "first message",
    "begrüßung", "greeting", "tags", "schlagwörter", "notiz", "notes", "creator notes", "descripción", "personalidad", "escenario", "имя", "описание", "личность", "сценарий",
];

/// The model's text tidied into the field's value. Err(`draft_empty`): nothing usable was written.
pub fn clean(field: Field, raw: &str) -> Result<Answer, &'static str> {
    // A thinking model's reasoning is never part of the answer.
    let mut text = raw;
    if let Some(end) = text.rfind("</think>") {
        text = &text[end + "</think>".len()..];
    }
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim_start().starts_with("```")).collect();
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    let mut text = lines.join("\n").trim().to_string();
    if field != Field::MesExample {
        text = strip_label(&text);
    }
    let text = text.trim();
    if text.is_empty() {
        return Err("draft_empty");
    }
    match field {
        Field::Name => {
            let first = text.lines().next().unwrap_or("");
            let name: String = first.trim_matches(|c: char| c == '*' || c == '"' || c == '„' || c == '“' || c == '«' || c == '»' || c == '.' || c.is_whitespace()).chars().take(60).collect();
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err("draft_empty");
            }
            Ok(Answer::Text(name))
        }
        Field::Tags => {
            let mut tags: Vec<String> = vec![];
            for part in text.split([',', '\n', ';', '、']) {
                let tag = part.trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')' || c == '-' || c == '*' || c == '#' || c.is_whitespace());
                let tag = tag.trim_matches(|c: char| c == '"' || c == '*' || c == '.' || c.is_whitespace());
                if tag.is_empty() || tag.chars().count() > 30 || tags.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
                    continue;
                }
                tags.push(tag.to_string());
                if tags.len() == 8 {
                    break;
                }
            }
            if tags.is_empty() {
                return Err("draft_empty");
            }
            Ok(Answer::List(tags))
        }
        Field::MesExample if !text.contains("<START>") => Ok(Answer::Text(format!("<START>\n{text}"))),
        _ => Ok(Answer::Text(text.to_string())),
    }
}

/// Takes a label off the start ("**Beschreibung:**", "Name: Mara", a label line of its own).
fn strip_label(text: &str) -> String {
    let Some(colon) = text.find(':') else { return text.to_string() };
    let label = text[..colon].trim().trim_matches(|c: char| c == '*' || c == '#' || c.is_whitespace()).to_lowercase();
    if !LABELS.contains(&label.as_str()) {
        return text.to_string();
    }
    text[colon + 1..].trim_start_matches(['*', ' ']).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Draft {
        Draft {
            name: " Ilse Brand ".into(),
            description: "Ilse keeps the lighthouse on a cold island.".into(),
            personality: "dry, patient".into(),
            scenario: "A storm night.".into(),
            first_mes: "*The door creaks.* \"You're late, {{user}}.\"".into(),
            mes_example: "<START>\n{{user}}: Hi.\n{{char}}: Hm.".into(),
            alternate_greetings: vec!["Morning.".into(), "  ".into()],
            tags: vec!["Drama".into(), "".into(), "Coast".into()],
            creator_notes: "A quiet slow-burn story.".into(),
            lore: vec![
                LoreEntry { name: "The lamp".into(), keys: vec!["lamp".into(), " light ".into()], content: "The lamp has burned for 90 years.".into(), ..Default::default() },
                LoreEntry { content: "  ".into(), ..Default::default() },
            ],
            creator: "Tester".into(),
            version: "1.0".into(),
            ..Default::default()
        }
    }

    #[test]
    fn card_json_is_v3_with_v2_fields_and_reads_back_the_same() {
        let d = sample();
        let raw = card_json(&d, 1_700_000_000);
        assert_eq!((raw["spec"].as_str(), raw["spec_version"].as_str()), (Some("chara_card_v3"), Some("3.0")));
        let data = &raw["data"];
        assert_eq!(data["name"], "Ilse Brand");
        assert_eq!(data["alternate_greetings"], json!(["Morning."]));
        assert_eq!(data["tags"], json!(["Drama", "Coast"]));
        assert_eq!(data["creation_date"], 1_700_000_000);
        let entries = data["character_book"]["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "an entry without text is left out");
        assert_eq!((entries[0]["keys"].clone(), entries[0]["enabled"].clone()), (json!(["lamp", "light"]), json!(true)));

        // The app's own parser takes it as it takes any imported card, through PNG-free JSON.
        let parsed = card::parse(raw.to_string().as_bytes()).unwrap();
        assert_eq!(parsed.card, d.card());
        let back = draft_from_raw(&parsed.raw).unwrap();
        assert_eq!(back.card(), d.card());
        assert_eq!(back.lore, vec![LoreEntry { name: "The lamp".into(), keys: vec!["lamp".into(), "light".into()], content: "The lamp has burned for 90 years.".into(), constant: false, enabled: true }]);
        assert_eq!((back.creator.as_str(), back.version.as_str()), ("Tester", "1.0"));
    }

    #[test]
    fn no_lorebook_without_entries() {
        let d = Draft { name: "Mara".into(), ..Default::default() };
        assert!(card_json(&d, 0)["data"].get("character_book").is_none());
    }

    #[test]
    fn draft_from_old_books_and_flat_cards() {
        let raw = json!({ "spec": "chara_card_v2", "data": { "name": "Mara", "character_book": { "entries": [
            { "key": ["harbour"], "comment": "Harbour", "content": "Fog every morning.", "enabled": false },
            { "keys": [], "content": "" },
        ] } } });
        let d = draft_from_raw(&raw).unwrap();
        assert_eq!(d.lore, vec![LoreEntry { name: "Harbour".into(), keys: vec!["harbour".into()], content: "Fog every morning.".into(), constant: false, enabled: false }]);
        assert_eq!(draft_from_raw(&json!({ "name": "Old", "first_mes": "hi" })).unwrap().first_mes, "hi");
        assert!(draft_from_raw(&json!({ "description": "no name" })).is_none());
    }

    #[test]
    fn prompt_holds_plan_and_other_fields_but_not_the_one_written() {
        let plan = Plan { idea: "A lighthouse keeper who hides a letter.".into(), relation: "{{user}} is the new assistant".into(), traits: vec!["dry".into(), " ".into(), "loyal".into()], ..Default::default() };
        let d = sample();
        let msgs = draft_messages(Language::En, &plan, &d, Field::Description);
        let user = &msgs[1].content;
        assert!(user.contains("Idee: A lighthouse keeper who hides a letter."));
        assert!(user.contains("Kernzüge: dry, loyal"));
        assert!(user.contains("Name: Ilse Brand"));
        assert!(!user.contains("Beschreibung: Ilse keeps"), "the field itself is written anew");
        assert!(user.ends_with(Field::Description.order()));
        assert!(msgs[0].content.contains("Write in English."));
        // Nothing written yet: no "so far" block.
        let empty = draft_messages(Language::De, &Plan::default(), &Draft::default(), Field::Name);
        assert!(!empty[1].content.contains("# Die Karte bisher"));
    }

    #[test]
    fn clean_tidies_what_models_write() {
        assert_eq!(clean(Field::Name, "**Name:** „Ilse Brand“.\nShe is…"), Ok(Answer::Text("Ilse Brand".into())));
        assert_eq!(clean(Field::Description, "<think>plan it</think>\n\nBeschreibung:\nIlse is tall."), Ok(Answer::Text("Ilse is tall.".into())));
        assert_eq!(clean(Field::Description, "```\nIlse is tall.\n```"), Ok(Answer::Text("Ilse is tall.".into())));
        assert_eq!(clean(Field::FirstMes, "Ilse: \"Hello.\""), Ok(Answer::Text("Ilse: \"Hello.\"".into())), "a speaker is not a label");
        assert_eq!(clean(Field::Tags, "1. Drama\n2. #Coast, drama, Slow Burn."), Ok(Answer::List(vec!["Drama".into(), "Coast".into(), "Slow Burn".into()])));
        assert_eq!(clean(Field::MesExample, "{{user}}: Hi.\n{{char}}: Hm."), Ok(Answer::Text("<START>\n{{user}}: Hi.\n{{char}}: Hm.".into())));
        assert_eq!(clean(Field::MesExample, "<START>\n{{char}}: Hm."), Ok(Answer::Text("<START>\n{{char}}: Hm.".into())));
        assert_eq!(clean(Field::Scenario, "<think>only thoughts</think>  "), Err("draft_empty"));
        assert_eq!(clean(Field::Tags, ", ,"), Err("draft_empty"));
        let many = (0..20).map(|i| format!("t{i}")).collect::<Vec<_>>().join(",");
        assert!(matches!(clean(Field::Tags, &many), Ok(Answer::List(l)) if l.len() == 8));
    }

    #[test]
    fn plan_and_draft_take_missing_fields() {
        let d: Draft = serde_json::from_value(json!({ "name": "Mara", "lore": [{ "content": "x" }] })).unwrap();
        assert!(d.lore[0].enabled, "an entry is on unless it says otherwise");
        let p: Plan = serde_json::from_value(json!({ "idea": "x" })).unwrap();
        assert_eq!(p.idea, "x");
        assert_eq!(serde_json::to_value(Field::FirstMes).unwrap(), "first_mes");
    }
}
