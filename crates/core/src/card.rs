//! Character card parsing: Tavern card V1 (flat JSON), V2 (`chara_card_v2`) and V3
//! (`chara_card_v3`), as plain JSON or embedded in a PNG tEXt chunk (`ccv3` preferred, then `chara`).

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The card fields the app uses. The original JSON is kept next to it for lossless export.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Card {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardError {
    /// Neither a PNG nor JSON.
    UnknownFormat,
    /// A PNG without `chara` / `ccv3` text chunk.
    NoCardInPng,
    /// Card data found but it is not valid base64 / JSON.
    BadData,
    /// JSON without a character name.
    NotACard,
}

impl CardError {
    /// Stable code for the API; the UI turns it into text.
    pub fn code(self) -> &'static str {
        match self {
            CardError::UnknownFormat => "unknown_format",
            CardError::NoCardInPng => "no_card_in_png",
            CardError::BadData => "bad_card_data",
            CardError::NotACard => "not_a_card",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub card: Card,
    /// The card JSON exactly as it was imported (V1 flat or V2/V3 with `spec` + `data`).
    pub raw: Value,
    /// The import was a PNG (it doubles as the avatar).
    pub is_png: bool,
}

pub(crate) const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

pub fn parse(bytes: &[u8]) -> Result<Imported, CardError> {
    if bytes.starts_with(&PNG_SIG) {
        let chunks = png_text_chunks(bytes);
        let find = |key: &str| chunks.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| *v);
        let text = find("ccv3").or_else(|| find("chara")).ok_or(CardError::NoCardInPng)?;
        let compact: Vec<u8> = text.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
        let json = base64::engine::general_purpose::STANDARD
            .decode(compact)
            .map_err(|_| CardError::BadData)?;
        let raw: Value = serde_json::from_slice(&json).map_err(|_| CardError::BadData)?;
        Ok(Imported { card: from_json(&raw)?, raw, is_png: true })
    } else {
        let text = std::str::from_utf8(bytes).map_err(|_| CardError::UnknownFormat)?;
        let text = text.trim_start_matches('\u{feff}').trim_start();
        if !text.starts_with('{') {
            return Err(CardError::UnknownFormat);
        }
        let raw: Value = serde_json::from_str(text).map_err(|_| CardError::BadData)?;
        Ok(Imported { card: from_json(&raw)?, raw, is_png: false })
    }
}

/// `(keyword, text)` of every tEXt chunk. Stops quietly at the first malformed chunk.
pub(crate) fn png_text_chunks(bytes: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = vec![];
    let mut pos = PNG_SIG.len();
    while let Some(head) = bytes.get(pos..pos + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = &head[4..8];
        let Some(data) = bytes.get(pos + 8..pos + 8 + len) else { break };
        if kind == b"tEXt" {
            if let Some(nul) = data.iter().position(|&b| b == 0) {
                out.push((String::from_utf8_lossy(&data[..nul]).into_owned(), &data[nul + 1..]));
            }
        } else if kind == b"IEND" {
            break;
        }
        pos += 12 + len; // length + type + data + crc
    }
    out
}

/// Normalizes V1 (fields at the top level) and V2/V3 (fields under `data`).
pub fn from_json(raw: &Value) -> Result<Card, CardError> {
    let data = if raw["data"].is_object() { &raw["data"] } else { raw };
    let s = |key: &str| data[key].as_str().unwrap_or("").trim().to_string();
    let list = |key: &str| -> Vec<String> {
        data[key]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect())
            .unwrap_or_default()
    };
    let card = Card {
        name: s("name"),
        description: s("description"),
        personality: s("personality"),
        scenario: s("scenario"),
        first_mes: s("first_mes"),
        mes_example: s("mes_example"),
        system_prompt: s("system_prompt"),
        post_history_instructions: s("post_history_instructions"),
        creator_notes: s("creator_notes"),
        alternate_greetings: list("alternate_greetings"),
        tags: list("tags"),
    };
    if card.name.is_empty() {
        return Err(CardError::NotACard);
    }
    Ok(card)
}

/// Writes edited fields back into the card JSON. Everything else in it (lorebook, extensions,
/// unknown fields) stays as it was, so an edited card still exports losslessly.
pub fn apply(raw: &mut Value, card: &Card) {
    if !raw.is_object() {
        *raw = Value::Object(Default::default());
    }
    let data = if raw["data"].is_object() { &mut raw["data"] } else { raw };
    for (key, value) in [
        ("name", &card.name),
        ("description", &card.description),
        ("personality", &card.personality),
        ("scenario", &card.scenario),
        ("first_mes", &card.first_mes),
        ("mes_example", &card.mes_example),
        ("system_prompt", &card.system_prompt),
        ("post_history_instructions", &card.post_history_instructions),
        ("creator_notes", &card.creator_notes),
    ] {
        data[key] = Value::String(value.clone());
    }
    data["alternate_greetings"] = card.alternate_greetings.iter().cloned().map(Value::String).collect();
    data["tags"] = card.tags.iter().cloned().map(Value::String).collect();
}

/// Builds a minimal PNG that carries `json` in a tEXt chunk (tests and fixtures only need the
/// chunk structure, not real image data).
#[cfg(test)]
pub(crate) fn png_with_chunk(keyword: &str, json: &str) -> Vec<u8> {
    let mut png = PNG_SIG.to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        png.extend_from_slice(&[0, 0, 0, 0]); // crc is not checked by the reader
    };
    chunk(b"IHDR", &[0; 13]);
    let b64 = base64::engine::general_purpose::STANDARD.encode(json);
    let mut text = keyword.as_bytes().to_vec();
    text.push(0);
    text.extend_from_slice(b64.as_bytes());
    chunk(b"tEXt", &text);
    chunk(b"IEND", &[]);
    png
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2: &str = r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{
        "name":"Mara","description":"A lighthouse keeper, 34 years old.","personality":"dry humour",
        "scenario":"A storm is coming.","first_mes":"Hello {{user}}.","mes_example":"",
        "system_prompt":"","post_history_instructions":"Stay calm.","creator_notes":"SFW sample",
        "alternate_greetings":["Oh. A visitor."],"tags":["Adventure"," "]}}"#;

    #[test]
    fn v2_json() {
        let imp = parse(V2.as_bytes()).unwrap();
        assert!(!imp.is_png);
        assert_eq!(imp.card.name, "Mara");
        assert_eq!(imp.card.post_history_instructions, "Stay calm.");
        assert_eq!(imp.card.alternate_greetings, vec!["Oh. A visitor."]);
        assert_eq!(imp.card.tags, vec!["Adventure"]);
    }

    #[test]
    fn v1_flat_json_with_bom() {
        let imp = parse("\u{feff}{\"name\":\"Old\",\"description\":\"d\",\"first_mes\":\"hi\"}".as_bytes()).unwrap();
        assert_eq!(imp.card.name, "Old");
        assert_eq!(imp.card.first_mes, "hi");
    }

    #[test]
    fn png_prefers_ccv3_over_chara() {
        let mut png = png_with_chunk("chara", r#"{"name":"V2 name"}"#);
        // Append a second card chunk before IEND by rebuilding: ccv3 first is not required.
        let v3 = png_with_chunk("ccv3", r#"{"spec":"chara_card_v3","data":{"name":"V3 name"}}"#);
        let iend = png.len() - 12;
        let v3_chunk = &v3[PNG_SIG.len() + 25..v3.len() - 12];
        png.splice(iend..iend, v3_chunk.iter().copied());
        let imp = parse(&png).unwrap();
        assert!(imp.is_png);
        assert_eq!(imp.card.name, "V3 name");
    }

    #[test]
    fn errors() {
        assert_eq!(parse(b"hello").unwrap_err(), CardError::UnknownFormat);
        assert_eq!(parse(b"{\"description\":\"no name\"}").unwrap_err(), CardError::NotACard);
        assert_eq!(parse(b"{ broken").unwrap_err(), CardError::BadData);
        let plain_png = [&PNG_SIG[..], &[0, 0, 0, 0], b"IEND", &[0, 0, 0, 0]].concat();
        assert_eq!(parse(&plain_png).unwrap_err(), CardError::NoCardInPng);
        let mut bad = png_with_chunk("chara", "{}");
        let at = bad.len() - 12 - 4 - 2; // inside the base64 text
        bad[at] = b'!';
        assert_eq!(parse(&bad).unwrap_err(), CardError::BadData);
    }

    #[test]
    fn apply_keeps_everything_it_does_not_edit() {
        let mut raw: Value = serde_json::from_str(
            r#"{"spec":"chara_card_v2","data":{"name":"Mara","description":"old","tags":["a"],
                "character_book":{"entries":[{"content":"lore"}]},"extensions":{"depth_prompt":{"depth":1}}}}"#,
        )
        .unwrap();
        let mut card = from_json(&raw).unwrap();
        card.description = "new".into();
        card.tags = vec!["b".into(), "c".into()];
        card.alternate_greetings = vec!["Hi".into()];
        apply(&mut raw, &card);
        assert_eq!(from_json(&raw).unwrap(), card);
        assert_eq!(raw["spec"], "chara_card_v2");
        assert_eq!(raw["data"]["character_book"]["entries"][0]["content"], "lore");
        assert_eq!(raw["data"]["extensions"]["depth_prompt"]["depth"], 1);

        let mut flat: Value = serde_json::from_str(r#"{"name":"Old","char_greeting":"x"}"#).unwrap();
        let mut v1 = from_json(&flat).unwrap();
        v1.name = "New".into();
        apply(&mut flat, &v1);
        assert_eq!(flat["name"], "New");
        assert_eq!(flat["char_greeting"], "x");
        assert!(flat.get("data").is_none());
    }

    #[test]
    fn truncated_png_does_not_panic() {
        let png = png_with_chunk("chara", r#"{"name":"X"}"#);
        for cut in 0..png.len() {
            let _ = parse(&png[..cut]);
        }
    }
}
