//! Every character card format the app imports, recognised by content (never by file name),
//! plus the writers for export: PNG with both `chara` (V2) and `ccv3` (V3) chunks, JSON V2/V3,
//! and the RisuAI persona PNG.
//!
//! Readers: PNG (`ccv3` before `chara`, `persona` = RisuAI persona), JSON V1/V2/V3, Pygmalion
//! and text-generation-webui JSON, YAML, CHARX (zip with `card.json`), Backyard `.byaf` (zip with
//! `manifest.json`), Perchance (gzip JSON, single share file or full Dexie export). Every reader
//! ends in a [`Card`] built by [`card::from_json`], so every format exposes the same fields as a
//! plain card.

use std::io::{Cursor, Read, Write};

use base64::Engine;
use serde_json::{json, Map, Value};

use crate::card::{self, Card, CardError, PNG_SIG};

/// Largest file accepted at all.
pub const MAX_INPUT: usize = 128 << 20;
/// Largest JSON (or YAML) text after base64 / gzip / zip decoding.
pub const MAX_JSON: u64 = 64 << 20;
/// Largest single image taken out of an archive or data URL.
pub const MAX_IMAGE: u64 = 32 << 20;
/// Most entries a zip may list (CHARX packs with sprites have a few hundred).
pub const MAX_ZIP_ENTRIES: usize = 10_000;
/// Most cards one file may carry (BYAF manifest, Perchance export).
pub const MAX_CARDS: usize = 1_000;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// PNG with a `ccv3` chunk (V3, wins over `chara`).
    PngCcv3,
    /// PNG with a `chara` chunk only (V1/V2, also Character Tavern `*.card.png`).
    PngChara,
    JsonV1,
    JsonV2,
    JsonV3,
    /// Old Pygmalion/Gradio JSON (`char_name`, `char_persona`, `char_greeting` …).
    JsonPygmalion,
    /// text-generation-webui JSON (`name`, `context`, `greeting`).
    JsonOoba,
    /// YAML card (text-generation-webui layout or Tavern fields).
    Yaml,
    /// CHARX zip (`card.json` V3 + assets), also when hidden behind a JPEG as RisuAI writes it.
    Charx,
    /// Backyard archive (`manifest.json`, `characters/*/character.json`, `scenarios/*.json`).
    Byaf,
    /// Perchance AI character chat: gzip JSON with `addCharacter`, or a Dexie database export.
    Perchance,
    /// RisuAI persona PNG (`persona` chunk) — a persona, not a card.
    RisuPersonaPng,
}

impl Format {
    /// Stable code for the API and logs.
    pub fn code(self) -> &'static str {
        match self {
            Format::PngCcv3 => "png_ccv3",
            Format::PngChara => "png_chara",
            Format::JsonV1 => "json_v1",
            Format::JsonV2 => "json_v2",
            Format::JsonV3 => "json_v3",
            Format::JsonPygmalion => "json_pygmalion",
            Format::JsonOoba => "json_ooba",
            Format::Yaml => "yaml",
            Format::Charx => "charx",
            Format::Byaf => "byaf",
            Format::Perchance => "perchance",
            Format::RisuPersonaPng => "risu_persona_png",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportError {
    /// Not a file type that can hold a card.
    UnknownFormat,
    /// A valid PNG / zip / JSON / YAML, but without card data (e.g. an ordinary picture).
    NoCard,
    /// Card data is there but damaged (bad base64, JSON, zip, gzip, missing name …).
    Broken,
    /// The file or something inside it is over the limits (zip / gzip bombs).
    TooLarge,
}

impl ImportError {
    /// Stable code for the API; the UI turns it into text.
    pub fn code(self) -> &'static str {
        match self {
            ImportError::UnknownFormat => "unknown_format",
            ImportError::NoCard => "no_card",
            ImportError::Broken => "broken",
            ImportError::TooLarge => "too_large",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl ImageKind {
    /// Recognises an image by its magic bytes.
    pub fn sniff(bytes: &[u8]) -> Option<ImageKind> {
        if bytes.starts_with(&PNG_SIG) {
            Some(ImageKind::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(ImageKind::Jpeg)
        } else if bytes.starts_with(b"GIF8") {
            Some(ImageKind::Gif)
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(ImageKind::Webp)
        } else {
            None
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            ImageKind::Png => "image/png",
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Webp => "image/webp",
            ImageKind::Gif => "image/gif",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub bytes: Vec<u8>,
    pub kind: ImageKind,
}

impl Image {
    fn from_bytes(bytes: Vec<u8>) -> Option<Image> {
        ImageKind::sniff(&bytes).map(|kind| Image { bytes, kind })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedCard {
    pub card: Card,
    /// Tavern card JSON (V1 flat, or V2/V3 with `spec` + `data`). Foreign formats are converted
    /// to V2, with what has no V2 field kept under `data.extensions`.
    pub raw: Value,
    /// Main avatar (for a PNG card: the file itself).
    pub image: Option<Image>,
}

/// A user persona in the RisuAI exchange format.
#[derive(Debug, Clone, PartialEq)]
pub struct Persona {
    pub name: String,
    pub prompt: String,
    pub note: String,
    /// The persona JSON as imported (unknown fields are written back on export).
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// One or more cards (BYAF and Perchance exports can hold several).
    Cards(Vec<ImportedCard>),
    Persona { persona: Persona, image: Option<Image> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedFile {
    pub format: Format,
    pub content: Content,
}

impl ImportedFile {
    fn cards(format: Format, cards: Vec<ImportedCard>) -> ImportedFile {
        ImportedFile { format, content: Content::Cards(cards) }
    }
}

/// Reads any supported card or persona file.
pub fn detect_and_parse(bytes: &[u8]) -> Result<ImportedFile, ImportError> {
    if bytes.len() > MAX_INPUT {
        return Err(ImportError::TooLarge);
    }
    if bytes.starts_with(&PNG_SIG) {
        return parse_png(bytes);
    }
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return parse_zip(bytes);
    }
    if bytes.starts_with(&[0x1F, 0x8B]) {
        let json = gunzip(bytes, MAX_JSON)?;
        let raw: Value = serde_json::from_slice(&json).map_err(|_| ImportError::Broken)?;
        return if is_perchance(&raw) { parse_perchance(&raw) } else { Err(ImportError::NoCard) };
    }
    if ImageKind::sniff(bytes).is_some() {
        // RisuAI also writes CHARX with a JPEG in front; a plain picture is no card.
        return match Zip::new(bytes) {
            Ok(mut zip) if zip.index_for_name("card.json").is_some() => parse_charx(&mut zip),
            _ => Err(ImportError::NoCard),
        };
    }
    parse_text(bytes)
}

// ---------------------------------------------------------------------------------------------
// PNG

fn parse_png(bytes: &[u8]) -> Result<ImportedFile, ImportError> {
    let chunks = card::png_text_chunks(bytes);
    let find = |key: &str| chunks.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| *v);
    let image = Some(Image { bytes: bytes.to_vec(), kind: ImageKind::Png });
    let (format, text) = match (find("ccv3"), find("chara")) {
        (Some(text), _) => (Format::PngCcv3, text),
        (None, Some(text)) => (Format::PngChara, text),
        (None, None) => {
            let text = find("persona").ok_or(ImportError::NoCard)?;
            let persona = persona_from_json(base64_json(text)?)?;
            return Ok(ImportedFile { format: Format::RisuPersonaPng, content: Content::Persona { persona, image } });
        }
    };
    let raw = base64_json(text)?;
    let card = card::from_json(&raw).map_err(|_| ImportError::Broken)?;
    Ok(ImportedFile::cards(format, vec![ImportedCard { card, raw, image }]))
}

fn base64_json(text: &[u8]) -> Result<Value, ImportError> {
    if text.len() as u64 > MAX_JSON / 3 * 4 + 4 {
        return Err(ImportError::TooLarge);
    }
    let compact: Vec<u8> = text.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    let json = B64.decode(compact).map_err(|_| ImportError::Broken)?;
    serde_json::from_slice(strip_bom(&json)).map_err(|_| ImportError::Broken)
}

fn persona_from_json(raw: Value) -> Result<Persona, ImportError> {
    let s = |key: &str| raw[key].as_str().unwrap_or("").trim().to_string();
    let (name, prompt, note) = (s("name"), s("personaPrompt"), s("note"));
    if name.is_empty() || raw["personaPrompt"].as_str().is_none() {
        return Err(ImportError::Broken);
    }
    Ok(Persona { name, prompt, note, raw })
}

// ---------------------------------------------------------------------------------------------
// Zip containers: CHARX and BYAF

/// A card zip being read and the bytes taken from it so far: all entries of one file together at
/// most [`MAX_UNPACKED`] (a BYAF manifest may name the same large entry a thousand times).
struct Zip<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    read: u64,
}

/// Most bytes all entries of one card zip may unpack to together.
pub const MAX_UNPACKED: u64 = 256 << 20;
/// Most bytes an entry above 1 MiB may unpack to per compressed byte (card JSON packs ~3–10×,
/// pictures ~1×; a zip bomb ~1000×).
pub const MAX_RATIO: u64 = 100;

impl<'a> Zip<'a> {
    fn new(bytes: &'a [u8]) -> Result<Zip<'a>, ImportError> {
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| ImportError::Broken)?;
        Ok(Zip { zip, read: 0 })
    }

    fn index_for_name(&self, name: &str) -> Option<usize> {
        self.zip.index_for_name(name)
    }
}

fn parse_zip(bytes: &[u8]) -> Result<ImportedFile, ImportError> {
    let mut zip = Zip::new(bytes)?;
    if zip.zip.len() > MAX_ZIP_ENTRIES {
        return Err(ImportError::TooLarge);
    }
    if zip.index_for_name("card.json").is_some() {
        parse_charx(&mut zip)
    } else if zip.index_for_name("manifest.json").is_some() {
        parse_byaf(&mut zip)
    } else {
        Err(ImportError::NoCard)
    }
}

/// Reads one entry, at most `limit` bytes (checked against the header and while inflating, so a
/// lying header does not help a zip bomb), within [`MAX_RATIO`] and what is left of
/// [`MAX_UNPACKED`]. `Ok(None)` when the entry does not exist.
fn read_entry(zip: &mut Zip, name: &str, limit: u64) -> Result<Option<Vec<u8>>, ImportError> {
    let mut file = match zip.zip.by_name(name) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(_) => return Err(ImportError::Broken),
    };
    let limit = limit.min(MAX_UNPACKED - zip.read);
    let size = file.size();
    if size > limit || (size > 1 << 20 && size / file.compressed_size().max(1) > MAX_RATIO) {
        return Err(ImportError::TooLarge);
    }
    let mut buf = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut buf).map_err(|_| ImportError::Broken)?;
    if buf.len() as u64 > limit {
        return Err(ImportError::TooLarge);
    }
    zip.read += buf.len() as u64;
    Ok(Some(buf))
}

fn read_json(zip: &mut Zip, name: &str) -> Result<Option<Value>, ImportError> {
    match read_entry(zip, name, MAX_JSON)? {
        Some(bytes) => serde_json::from_slice(strip_bom(&bytes)).map(Some).map_err(|_| ImportError::Broken),
        None => Ok(None),
    }
}

fn read_image(zip: &mut Zip, name: &str) -> Result<Option<Image>, ImportError> {
    Ok(read_entry(zip, name, MAX_IMAGE)?.and_then(Image::from_bytes))
}

fn parse_charx(zip: &mut Zip) -> Result<ImportedFile, ImportError> {
    let raw = read_json(zip, "card.json")?.ok_or(ImportError::NoCard)?;
    let card = card::from_json(&raw).map_err(|_| ImportError::Broken)?;
    // Main avatar: icon asset named "main", else the first icon. `embeded://` is the spec's spelling.
    let assets = raw["data"]["assets"].as_array().cloned().unwrap_or_default();
    let icons: Vec<&Value> = assets.iter().filter(|a| a["type"] == "icon").collect();
    let main = icons.iter().find(|a| a["name"] == "main").or(icons.first());
    let path = main
        .and_then(|a| a["uri"].as_str())
        .and_then(|uri| uri.strip_prefix("embeded://").or_else(|| uri.strip_prefix("embedded://")));
    let image = match path {
        Some(path) => read_image(zip, path)?,
        None => None,
    };
    Ok(ImportedFile::cards(Format::Charx, vec![ImportedCard { card, raw, image }]))
}

fn parse_byaf(zip: &mut Zip) -> Result<ImportedFile, ImportError> {
    let manifest = read_json(zip, "manifest.json")?.ok_or(ImportError::NoCard)?;
    let Some(characters) = manifest["characters"].as_array().filter(|c| !c.is_empty()) else {
        return Err(ImportError::NoCard);
    };
    if characters.len() > MAX_CARDS {
        return Err(ImportError::TooLarge);
    }
    // Like SillyTavern: scenarios that are missing or damaged are skipped.
    let mut scenarios = vec![];
    for path in manifest["scenarios"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if let Ok(Some(scenario)) = read_json(zip, path) {
            scenarios.push(scenario);
        }
    }
    let mut cards = vec![];
    for path in characters {
        let path = path.as_str().ok_or(ImportError::Broken)?;
        let character = read_json(zip, path)?.ok_or(ImportError::Broken)?;
        let raw = byaf_card(&manifest, &character, &scenarios);
        let card = card::from_json(&raw).map_err(|_| ImportError::Broken)?;
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut image = None;
        for rel in character["images"].as_array().into_iter().flatten().filter_map(|i| i["path"].as_str()) {
            let full = if dir.is_empty() { rel.to_string() } else { format!("{dir}/{rel}") };
            if let Some(found) = read_image(zip, &full)? {
                image = Some(found);
                break;
            }
        }
        cards.push(ImportedCard { card, raw, image });
    }
    Ok(ImportedFile::cards(Format::Byaf, cards))
}

/// BYAF character + scenarios as a V2 card, mapped the way SillyTavern does it.
fn byaf_card(manifest: &Value, character: &Value, scenarios: &[Value]) -> Value {
    let text = |v: &Value| byaf_macros(v.as_str().unwrap_or(""));
    let first = scenarios.first().unwrap_or(&Value::Null);
    let first_mes = text(&first["firstMessages"][0]["text"]);
    let mut greetings: Vec<String> = vec![];
    for scenario in scenarios.iter().skip(1) {
        let greeting = text(&scenario["firstMessages"][0]["text"]);
        if !greeting.is_empty() && greeting != first_mes && !greetings.contains(&greeting) {
            greetings.push(greeting);
        }
    }
    let examples: Vec<String> = first["exampleMessages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["text"].as_str().filter(|t| !t.is_empty()))
        .map(|t| format!("<START>\n{}", byaf_macros(t)))
        .collect();
    let name = character["name"].as_str().filter(|n| !n.trim().is_empty()).or(character["displayName"].as_str());
    let mut flat = json!({
        "name": name.unwrap_or(""),
        "description": text(&character["persona"]),
        "scenario": text(&first["narrative"]),
        "first_mes": first_mes,
        "mes_example": examples.join("\n"),
        "creator_notes": manifest["author"]["backyardURL"].as_str().unwrap_or(""),
        "system_prompt": text(&first["formattingInstructions"]),
        "alternate_greetings": greetings,
        "tags": if character["isNSFW"] == true { vec!["nsfw"] } else { vec![] },
        "creator": manifest["author"]["name"].as_str().unwrap_or(""),
        "extensions": {},
    });
    if let Some(display) = character["displayName"].as_str() {
        flat["extensions"]["display_name"] = json!(display);
    }
    let lore: Vec<Value> = character["loreItems"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, item)| {
            let keys: Vec<String> =
                text(&item["key"]).split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
            json!({ "keys": keys, "content": text(&item["value"]), "extensions": {}, "enabled": true, "insertion_order": i })
        })
        .collect();
    if !lore.is_empty() {
        flat["character_book"] = json!({ "entries": lore, "extensions": {} });
    }
    to_spec(&flat, V2)
}

/// Backyard placeholders → Tavern macros (`{user}`, `{character}`, `#{user}:` line prefixes).
fn byaf_macros(text: &str) -> String {
    const TOKENS: [(&str, &str, bool); 4] = [
        ("#{user}:", "{{user}}:", false),
        ("#{character}:", "{{char}}:", false),
        ("{character}", "{{char}}", true),
        ("{user}", "{{user}}", true),
    ];
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < bytes.len() {
        for (token, replacement, not_before_brace) in TOKENS {
            let end = i + token.len();
            if bytes.get(i..end).is_some_and(|s| s.eq_ignore_ascii_case(token.as_bytes()))
                && !(not_before_brace && bytes.get(end) == Some(&b'}'))
            {
                out.push_str(replacement);
                i = end;
                continue 'scan;
            }
        }
        // Tokens are ASCII, so `i` is always on a char boundary here.
        let Some(ch) = text[i..].chars().next() else { break };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

// ---------------------------------------------------------------------------------------------
// gzip: Perchance

fn gunzip(bytes: &[u8], limit: u64) -> Result<Vec<u8>, ImportError> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).take(limit + 1).read_to_end(&mut out).map_err(|_| ImportError::Broken)?;
    if out.len() as u64 > limit {
        return Err(ImportError::TooLarge);
    }
    Ok(out)
}

fn is_perchance(raw: &Value) -> bool {
    raw["addCharacter"].is_object() || raw["formatName"] == "dexie"
}

fn parse_perchance(raw: &Value) -> Result<ImportedFile, ImportError> {
    let rows: Vec<&Value> = if raw["addCharacter"].is_object() {
        vec![&raw["addCharacter"]]
    } else {
        // Dexie export of the whole app: data.data[] = { tableName, rows }.
        raw["data"]["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["tableName"] == "characters")
            .flat_map(|t| t["rows"].as_array().into_iter().flatten())
            .collect()
    };
    if rows.is_empty() {
        return Err(ImportError::NoCard);
    }
    if rows.len() > MAX_CARDS {
        return Err(ImportError::TooLarge);
    }
    let cards = rows.into_iter().map(perchance_card).collect::<Result<Vec<_>, _>>()?;
    Ok(ImportedFile::cards(Format::Perchance, cards))
}

fn perchance_card(c: &Value) -> Result<ImportedCard, ImportError> {
    let s = |key: &str| c[key].as_str().unwrap_or("");
    let avatar = c["avatar"]["url"].as_str().or(c["avatar"].as_str()).unwrap_or("");
    let (image, avatar_url) = match avatar.strip_prefix("data:image/") {
        Some(rest) => {
            let b64 = rest.split_once(',').map_or("", |(_, data)| data);
            if b64.len() as u64 > MAX_IMAGE / 3 * 4 + 4 {
                return Err(ImportError::TooLarge);
            }
            (B64.decode(b64.trim()).ok().and_then(Image::from_bytes), Value::Null)
        }
        None if avatar.is_empty() => (None, Value::Null),
        None => (None, json!(avatar)),
    };
    // First greeting the AI writes (Perchance `initialMessages`), if any.
    let first_mes = c["initialMessages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["author"] == "ai")
        .and_then(|m| m["content"].as_str())
        .unwrap_or("");
    let flat = json!({
        "name": if s("name").trim().is_empty() { "Unnamed Perchance Character" } else { s("name") },
        "description": s("roleInstruction"),
        "first_mes": first_mes,
        // The reminder is injected right before each reply, which is what post-history instructions do.
        "post_history_instructions": s("reminderMessage"),
        "creator": s("metaTitle"),
        "creator_notes": s("metaDescription"),
        "extensions": { "perchance_data": {
            "uuid": c["uuid"].clone(),
            "avatar_url": avatar_url,
            "folder_path": c["folderPath"].clone(),
            "custom_data": c["customData"].clone(),
        }},
    });
    let raw = to_spec(&flat, V2);
    let card = card::from_json(&raw).map_err(|_| ImportError::Broken)?;
    Ok(ImportedCard { card, raw, image })
}

// ---------------------------------------------------------------------------------------------
// Text: JSON and YAML

fn parse_text(bytes: &[u8]) -> Result<ImportedFile, ImportError> {
    if bytes.len() as u64 > MAX_JSON {
        return Err(ImportError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ImportError::UnknownFormat)?;
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with('{') {
        let raw: Value = serde_json::from_str(text).map_err(|_| ImportError::Broken)?;
        return parse_object(raw, None);
    }
    let value = parse_yaml(text)?;
    parse_object(value, Some(Format::Yaml))
}

/// A JSON object (or a YAML mapping turned into one). `force` overrides the reported format.
fn parse_object(raw: Value, force: Option<Format>) -> Result<ImportedFile, ImportError> {
    if !raw.is_object() {
        return Err(ImportError::NoCard);
    }
    if is_perchance(&raw) {
        return parse_perchance(&raw);
    }
    let has = |key: &str| raw.get(key).is_some();
    let (format, raw) = if !has("name") && !has("data") && raw["char_name"].is_string() {
        (Format::JsonPygmalion, pygmalion_card(&raw))
    } else if !has("data") && !has("description") && !has("first_mes") && (has("context") || has("greeting")) {
        (Format::JsonOoba, ooba_card(&raw))
    } else {
        let format = match raw["spec"].as_str() {
            Some("chara_card_v3") => Format::JsonV3,
            Some("chara_card_v2") => Format::JsonV2,
            _ if raw["data"].is_object() => Format::JsonV2,
            _ => Format::JsonV1,
        };
        (format, raw)
    };
    let card = card::from_json(&raw).map_err(|e| match e {
        CardError::NotACard => ImportError::NoCard,
        _ => ImportError::Broken,
    })?;
    Ok(ImportedFile::cards(force.unwrap_or(format), vec![ImportedCard { card, raw, image: None }]))
}

fn pygmalion_card(raw: &Value) -> Value {
    let s = |key: &str| raw[key].clone();
    let mut flat = json!({
        "name": s("char_name"),
        "description": s("char_persona"),
        "first_mes": s("char_greeting"),
        "mes_example": s("example_dialogue"),
        "scenario": s("world_scenario"),
        "creator_notes": if raw["creatorcomment"].is_string() { s("creatorcomment") } else { s("creator_notes") },
        "creator": s("creator"),
    });
    if raw["tags"].is_array() {
        flat["tags"] = s("tags");
    }
    to_spec(&strip_nulls(flat), V2)
}

/// text-generation-webui character (`name`, `context`, `greeting`, optional `example_dialogue`).
fn ooba_card(raw: &Value) -> Value {
    let flat = json!({
        "name": raw["name"].clone(),
        "description": raw["context"].clone(),
        "first_mes": raw["greeting"].clone(),
        "mes_example": raw["example_dialogue"].clone(),
    });
    to_spec(&strip_nulls(flat), V2)
}

fn strip_nulls(mut v: Value) -> Value {
    if let Some(map) = v.as_object_mut() {
        map.retain(|_, v| !v.is_null());
    }
    v
}

/// Parses one YAML mapping. Aliases are refused: they are expanded by copying, which lets a few
/// bytes grow into gigabytes ("billion laughs"), and real cards never use them.
fn parse_yaml(text: &str) -> Result<Value, ImportError> {
    use yaml_rust2::{parser::Parser, Event, Yaml, YamlLoader};
    let mut parser = Parser::new_from_str(text);
    loop {
        match parser.next_token() {
            Ok((Event::StreamEnd, _)) => break,
            Ok((Event::Alias(_), _)) => return Err(ImportError::Broken),
            Ok(_) => {}
            Err(_) => return Err(ImportError::UnknownFormat),
        }
    }
    let docs = YamlLoader::load_from_str(text).map_err(|_| ImportError::UnknownFormat)?;
    fn to_json(y: &Yaml) -> Value {
        match y {
            Yaml::String(s) | Yaml::Real(s) => Value::String(s.clone()),
            Yaml::Integer(i) => json!(i),
            Yaml::Boolean(b) => json!(b),
            Yaml::Array(items) => items.iter().map(to_json).collect(),
            Yaml::Hash(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let key = match k {
                            Yaml::String(s) | Yaml::Real(s) => s.clone(),
                            Yaml::Integer(i) => i.to_string(),
                            Yaml::Boolean(b) => b.to_string(),
                            _ => String::new(),
                        };
                        (key, to_json(v))
                    })
                    .collect(),
            ),
            Yaml::Null | Yaml::Alias(_) | Yaml::BadValue => Value::Null,
        }
    }
    match docs.first() {
        Some(doc @ Yaml::Hash(_)) => Ok(to_json(doc)),
        _ => Err(ImportError::UnknownFormat),
    }
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

// ---------------------------------------------------------------------------------------------
// Writers

#[derive(Clone, Copy)]
struct Spec {
    spec: &'static str,
    version: &'static str,
    v3: bool,
}

const V2: Spec = Spec { spec: "chara_card_v2", version: "2.0", v3: false };
const V3: Spec = Spec { spec: "chara_card_v3", version: "3.0", v3: true };

/// The card as V2 (`chara_card_v2`). A V1 card is wrapped; all other fields, also V3-only ones
/// and unknown ones, are kept. Missing required V2 fields are added empty.
pub fn to_v2(raw: &Value) -> Value {
    to_spec(raw, V2)
}

/// The card as V3 (`chara_card_v3`), same rules as [`to_v2`] plus `group_only_greetings`.
pub fn to_v3(raw: &Value) -> Value {
    to_spec(raw, V3)
}

fn to_spec(raw: &Value, spec: Spec) -> Value {
    let wrapped = raw["data"].is_object();
    let mut data = match raw.as_object() {
        Some(_) if wrapped => raw["data"].as_object().cloned().unwrap_or_default(),
        Some(flat) => flat.clone(),
        None => Map::new(),
    };
    for key in [
        "name",
        "description",
        "personality",
        "scenario",
        "first_mes",
        "mes_example",
        "creator_notes",
        "system_prompt",
        "post_history_instructions",
        "creator",
        "character_version",
    ] {
        data.entry(key).or_insert_with(|| json!(""));
    }
    data.entry("alternate_greetings").or_insert_with(|| json!([]));
    data.entry("tags").or_insert_with(|| json!([]));
    data.entry("extensions").or_insert_with(|| json!({}));
    if spec.v3 {
        data.entry("group_only_greetings").or_insert_with(|| json!([]));
    }
    let mut out = if wrapped { raw.as_object().cloned().unwrap_or_default() } else { Map::new() };
    out.insert("spec".into(), json!(spec.spec));
    out.insert("spec_version".into(), json!(spec.version));
    out.insert("data".into(), Value::Object(data));
    Value::Object(out)
}

/// Card as pretty V2 JSON.
pub fn write_json_v2(raw: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(&to_v2(raw)).expect("a JSON value always serializes")
}

/// Card as pretty V3 JSON.
pub fn write_json_v3(raw: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(&to_v3(raw)).expect("a JSON value always serializes")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteError {
    /// The picture to write into is not a PNG (convert JPEG/WebP first).
    ImageNotPng,
    /// The PNG is truncated or its chunks do not add up.
    BrokenImage,
}

/// Card PNG with both chunks, the way SillyTavern writes it: `chara` = V2, `ccv3` = V3 of the
/// same data. Uses `image` (old card chunks in it are replaced) or a neutral placeholder.
pub fn write_png(raw: &Value, image: Option<&[u8]>) -> Result<Vec<u8>, WriteError> {
    let v2 = serde_json::to_vec(&to_v2(raw)).expect("a JSON value always serializes");
    let v3 = serde_json::to_vec(&to_v3(raw)).expect("a JSON value always serializes");
    embed_text(image, &["chara", "ccv3"], &[("chara", &v2), ("ccv3", &v3)])
}

/// RisuAI persona PNG: chunk `persona` = base64 JSON `{name, personaPrompt, note}` (plus any
/// other fields that came with the import). Without a picture Risu's grey square is used.
pub fn write_persona_png(persona: &Persona, image: Option<&[u8]>) -> Result<Vec<u8>, WriteError> {
    let mut raw = persona.raw.as_object().cloned().unwrap_or_default();
    raw.insert("name".into(), json!(persona.name));
    raw.insert("personaPrompt".into(), json!(persona.prompt));
    raw.insert("note".into(), json!(persona.note));
    let json = serde_json::to_vec(&Value::Object(raw)).expect("a JSON value always serializes");
    embed_text(image, &["persona"], &[("persona", &json)])
}

/// 256×256 neutral grey PNG (the size and colour RisuAI uses when a persona has no picture).
pub fn placeholder_png() -> Vec<u8> {
    const SIDE: u32 = 256;
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&SIDE.to_be_bytes());
    ihdr.extend_from_slice(&SIDE.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // 8-bit greyscale, deflate, no filter, no interlace
    let mut rows = Vec::with_capacity(((SIDE + 1) * SIDE) as usize);
    for _ in 0..SIDE {
        rows.push(0); // filter: none
        rows.extend(std::iter::repeat_n(0x80, SIDE as usize));
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&rows).expect("writing to a Vec cannot fail");
    let idat = z.finish().expect("writing to a Vec cannot fail");
    let mut png = PNG_SIG.to_vec();
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &idat);
    push_chunk(&mut png, b"IEND", &[]);
    png
}

/// Copies the PNG chunk by chunk, drops tEXt chunks whose keyword is in `drop`, and adds `texts`
/// (base64 encoded) right before IEND.
fn embed_text(image: Option<&[u8]>, drop: &[&str], texts: &[(&str, &[u8])]) -> Result<Vec<u8>, WriteError> {
    let placeholder;
    let png = match image {
        Some(bytes) => bytes,
        None => {
            placeholder = placeholder_png();
            placeholder.as_slice()
        }
    };
    if !png.starts_with(&PNG_SIG) {
        return Err(WriteError::ImageNotPng);
    }
    let mut out = PNG_SIG.to_vec();
    let mut pos = PNG_SIG.len();
    loop {
        let head = png.get(pos..pos + 8).ok_or(WriteError::BrokenImage)?;
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind: [u8; 4] = [head[4], head[5], head[6], head[7]];
        let end = pos.checked_add(12 + len).ok_or(WriteError::BrokenImage)?;
        let whole = png.get(pos..end).ok_or(WriteError::BrokenImage)?;
        let data = &whole[8..8 + len];
        if &kind == b"IEND" {
            for (keyword, json) in texts {
                let mut text = keyword.as_bytes().to_vec();
                text.push(0);
                text.extend_from_slice(B64.encode(json).as_bytes());
                push_chunk(&mut out, b"tEXt", &text);
            }
            push_chunk(&mut out, b"IEND", &[]);
            return Ok(out);
        }
        let dropped = &kind == b"tEXt"
            && data
                .iter()
                .position(|&b| b == 0)
                .is_some_and(|nul| drop.iter().any(|d| d.as_bytes().eq_ignore_ascii_case(&data[..nul])));
        if !dropped {
            out.extend_from_slice(whole);
        }
        pos = end;
    }
}

fn push_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    png.extend_from_slice(&crc.sum().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARA_JSON: &[u8] = include_bytes!("../../../tests/fixtures/mara_voss.json");
    const MOOS_JSON: &[u8] = include_bytes!("../../../tests/fixtures/moos.json");
    const ILSE_PNG: &[u8] = include_bytes!("../../../tests/fixtures/ilse_brandt.png");
    const CHARX: &[u8] = include_bytes!("../../../tests/fixtures/formats/moos.charx");
    const YAML: &[u8] = include_bytes!("../../../tests/fixtures/formats/mara_voss.yaml");
    const BYAF: &[u8] = include_bytes!("../../../tests/fixtures/formats/ilse_brandt.byaf");
    const PERCHANCE: &[u8] = include_bytes!("../../../tests/fixtures/formats/mara_voss.perchance.gz");
    const PERSONA: &[u8] = include_bytes!("../../../tests/fixtures/formats/persona_risu.png");

    fn single(bytes: &[u8]) -> (Format, ImportedCard) {
        let file = detect_and_parse(bytes).unwrap();
        match file.content {
            Content::Cards(mut cards) if cards.len() == 1 => (file.format, cards.remove(0)),
            other => panic!("expected one card, got {other:?}"),
        }
    }

    /// read → write PNG → read gives the same card; a second round gives the same bytes of JSON.
    /// Also JSON V2 and V3 round trips, and the `chara` chunk alone carries the same card.
    fn round_trip(imported: &ImportedCard) {
        let png_image = imported.image.as_ref().filter(|i| i.kind == ImageKind::Png).map(|i| i.bytes.as_slice());
        let png = write_png(&imported.raw, png_image).unwrap();
        check_png_crcs(&png, true);
        let (format, again) = single(&png);
        assert_eq!(format, Format::PngCcv3);
        assert_eq!(again.card, imported.card);
        assert_eq!(again.raw, to_v3(&imported.raw));
        // From the first export on, read → write gives byte-identical files.
        let png2 = write_png(&again.raw, Some(&png)).unwrap();
        let back = single(&png2).1;
        assert_eq!((&back.card, &back.raw), (&again.card, &again.raw));
        assert_eq!(write_png(&back.raw, Some(&png2)).unwrap(), png2);
        assert_eq!(image_chunks(&png2), image_chunks(&png));

        let chunks = card::png_text_chunks(&png);
        let chara = chunks.iter().find(|(k, _)| k == "chara").unwrap().1;
        let v2 = base64_json(chara).unwrap();
        assert_eq!(v2["spec"], "chara_card_v2");
        assert_eq!(card::from_json(&v2).unwrap(), imported.card);
        assert_eq!(chunks.iter().filter(|(k, _)| k == "chara" || k == "ccv3").count(), 2);

        for (bytes, format) in [(write_json_v2(&imported.raw), Format::JsonV2), (write_json_v3(&imported.raw), Format::JsonV3)] {
            let (f, json) = single(&bytes);
            assert_eq!(f, format);
            assert_eq!(json.card, imported.card);
            let rewritten = if format == Format::JsonV2 { write_json_v2(&json.raw) } else { write_json_v3(&json.raw) };
            assert_eq!(rewritten, bytes);
        }
    }

    fn image_chunks(png: &[u8]) -> Vec<&[u8]> {
        let mut out = vec![];
        let mut pos = 8;
        while pos < png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            if &png[pos + 4..pos + 8] != b"tEXt" {
                out.push(&png[pos..pos + 12 + len]);
            }
            pos += 12 + len;
        }
        out
    }

    /// Checks the CRC of every chunk, or with `text_only` only of the tEXt chunks the writer made
    /// (other chunks are copied as they came, and test PNGs from `png_with_chunk` carry zero CRCs).
    fn check_png_crcs(png: &[u8], text_only: bool) {
        let mut pos = 8;
        while pos < png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            if text_only && &png[pos + 4..pos + 8] != b"tEXt" {
                pos += 12 + len;
                continue;
            }
            let mut crc = flate2::Crc::new();
            crc.update(&png[pos + 4..pos + 8 + len]);
            assert_eq!(crc.sum().to_be_bytes(), png[pos + 8 + len..pos + 12 + len]);
            pos += 12 + len;
        }
        assert_eq!(pos, png.len());
    }

    #[test]
    fn json_v1_v2_v3() {
        let (format, mara) = single(MARA_JSON);
        assert_eq!(format, Format::JsonV2);
        assert_eq!(mara.card.name, "Mara Voss");
        round_trip(&mara);

        let (format, moos) = single(&write_json_v3(&serde_json::from_slice(MOOS_JSON).unwrap()));
        assert_eq!(format, Format::JsonV3);
        assert_eq!(moos.card.name, "Moos");
        round_trip(&moos);

        let (format, v1) = single(b"\xEF\xBB\xBF{\"name\":\"Old\",\"description\":\"d\",\"first_mes\":\"hi\"}");
        assert_eq!(format, Format::JsonV1);
        assert_eq!(v1.card.first_mes, "hi");
        round_trip(&v1);
    }

    #[test]
    fn png_ccv3_and_chara() {
        let (format, ilse) = single(ILSE_PNG);
        assert_eq!(format, Format::PngCcv3);
        assert_eq!(ilse.card.name, "Kapitän Ilse Brandt");
        assert_eq!(ilse.image.as_ref().unwrap().bytes, ILSE_PNG);
        round_trip(&ilse);
        // The written PNG keeps the picture: every non-text chunk is copied unchanged.
        let png = write_png(&ilse.raw, Some(ILSE_PNG)).unwrap();
        assert_eq!(image_chunks(&png), image_chunks(ILSE_PNG));

        // `chara` only (V2 data, like a Character Tavern *.card.png).
        let chara_only = card::png_with_chunk("chara", std::str::from_utf8(MARA_JSON).unwrap());
        let (format, mara) = single(&chara_only);
        assert_eq!(format, Format::PngChara);
        assert_eq!(mara.card.name, "Mara Voss");
        round_trip(&mara);
    }

    #[test]
    fn charx() {
        let (format, moos) = single(CHARX);
        assert_eq!(format, Format::Charx);
        assert_eq!(moos.card.name, "Moos");
        assert_eq!(moos.raw["spec"], "chara_card_v3");
        let image = moos.image.as_ref().unwrap();
        assert_eq!(image.kind, ImageKind::Png);
        assert_eq!(&image.bytes[16..24], &[0, 0, 0, 48, 0, 0, 0, 48], "the main icon, not the background");
        round_trip(&moos);
        // Assets list survives the PNG export.
        let png = write_png(&moos.raw, Some(&image.bytes)).unwrap();
        assert_eq!(single(&png).1.raw["data"]["assets"], moos.raw["data"]["assets"]);

        // RisuAI: CHARX behind a JPEG header.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10];
        jpeg.extend_from_slice(CHARX);
        let (format, hidden) = single(&jpeg);
        assert_eq!(format, Format::Charx);
        assert_eq!(hidden.card, moos.card);
    }

    #[test]
    fn yaml() {
        let (format, mara) = single(YAML);
        assert_eq!(format, Format::Yaml);
        assert_eq!(mara.card.name, "Mara Voss");
        assert!(mara.card.description.contains("Leuchtturmwärterin"));
        assert!(mara.card.first_mes.starts_with("*Die Tür"));
        assert!(mara.card.mes_example.contains("Drei Tage"));
        round_trip(&mara);

        let tavern_yaml = "spec: chara_card_v2\ndata:\n  name: Moos\n  description: Waldgeist\n  tags: [Fantasy, Wald]\n";
        let (format, moos) = single(tavern_yaml.as_bytes());
        assert_eq!(format, Format::Yaml);
        assert_eq!(moos.card.tags, vec!["Fantasy", "Wald"]);
        round_trip(&moos);

        let bomb = "a: &a [x, x, x]\nb: &b [*a, *a, *a]\nname: *b\n";
        assert_eq!(detect_and_parse(bomb.as_bytes()).unwrap_err(), ImportError::Broken);
        assert_eq!(detect_and_parse(b"title: no card here\n").unwrap_err(), ImportError::NoCard);
    }

    #[test]
    fn byaf() {
        let (format, ilse) = single(BYAF);
        assert_eq!(format, Format::Byaf);
        assert_eq!(ilse.card.name, "Ilse");
        assert_eq!(ilse.raw["data"]["extensions"]["display_name"], "Kapitän Ilse Brandt");
        assert!(ilse.card.description.starts_with("{{char}}, 52 Jahre"));
        assert!(ilse.card.first_mes.contains("Du bist {{user}}"));
        assert!(ilse.card.scenario.starts_with("{{user}} heuert"));
        assert_eq!(ilse.card.alternate_greetings, vec!["*Ilse sieht von der Seekarte auf.* \"Wieder du, {{user}}?\""]);
        assert_eq!(ilse.card.mes_example, "<START>\n{{user}}: Wann legen wir ab?\n{{char}}: \"Mit der Flut. Nicht früher.\"");
        assert_eq!(ilse.card.system_prompt, "Schreibe in der dritten Person.");
        assert_eq!(ilse.raw["data"]["character_book"]["entries"][0]["keys"], json!(["Windsbraut", "Schiff"]));
        assert_eq!(ilse.image.as_ref().unwrap().kind, ImageKind::Png);
        round_trip(&ilse);
    }

    /// A BYAF manifest that names the same 1 MB character 300 times: stopped at MAX_UNPACKED
    /// instead of unpacking 300 MB (the same with 1000 names and 64 MB would be 64 GB).
    #[test]
    fn byaf_repeating_one_entry_is_no_bomb() {
        let mut src = zip::ZipArchive::new(Cursor::new(BYAF)).unwrap();
        let mut files: Vec<(String, Vec<u8>)> = (0..src.len())
            .map(|i| {
                let mut f = src.by_index(i).unwrap();
                let mut buf = vec![];
                f.read_to_end(&mut buf).unwrap();
                (f.name().to_string(), buf)
            })
            .collect();
        let manifest: Value = serde_json::from_slice(&files.iter().find(|(n, _)| n == "manifest.json").unwrap().1).unwrap();
        let path = manifest["characters"][0].as_str().unwrap().to_string();
        // About 1 MiB of text that does not pack well (stays under the ratio limit).
        let mut x = 7u32;
        let noise: String = (0..1 << 20).map(|_| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            char::from(b'a' + (x >> 16) as u8 % 26)
        }).collect();
        for (name, bytes) in &mut files {
            if *name == path {
                let mut c: Value = serde_json::from_slice(bytes).unwrap();
                c["padding"] = json!(noise);
                *bytes = serde_json::to_vec(&c).unwrap();
            } else if name == "manifest.json" {
                let mut m = manifest.clone();
                m["characters"] = json!(vec![path.clone(); 300]);
                *bytes = serde_json::to_vec(&m).unwrap();
            }
        }
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        assert_eq!(detect_and_parse(&zip_of(&refs)).unwrap_err(), ImportError::TooLarge);
    }

    #[test]
    fn byaf_macros_match_sillytavern() {
        assert_eq!(byaf_macros("#{user}: hi\n#{Character}: ho"), "{{user}}: hi\n{{char}}: ho");
        assert_eq!(byaf_macros("{user} and {CHARACTER} but {{user}} stays"), "{{user}} and {{char}} but {{user}} stays");
        assert_eq!(byaf_macros("Grüße, {user}!"), "Grüße, {{user}}!");
    }

    #[test]
    fn perchance() {
        let (format, mara) = single(PERCHANCE);
        assert_eq!(format, Format::Perchance);
        assert_eq!(mara.card.name, "Mara Voss");
        assert!(mara.card.first_mes.starts_with("*Die Tür"));
        assert!(!mara.card.post_history_instructions.is_empty());
        assert_eq!(mara.image.as_ref().unwrap().kind, ImageKind::Png);
        assert!(mara.raw["data"]["extensions"]["perchance_data"]["avatar_url"].is_null());
        round_trip(&mara);

        // Full Dexie export with two characters, gzipped.
        let export = json!({"formatName": "dexie", "formatVersion": 1, "data": {"data": [
            {"tableName": "messages", "rows": [{"message": "x"}]},
            {"tableName": "characters", "rows": [
                {"name": "Moos", "roleInstruction": "Waldgeist", "avatar": {"url": "https://example.org/moos.webp"}},
                {"name": "Mara", "roleInstruction": "Leuchtturm"}]}]}});
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(export.to_string().as_bytes()).unwrap();
        let file = detect_and_parse(&gz.finish().unwrap()).unwrap();
        assert_eq!(file.format, Format::Perchance);
        let Content::Cards(cards) = file.content else { panic!() };
        assert_eq!(cards.iter().map(|c| c.card.name.as_str()).collect::<Vec<_>>(), ["Moos", "Mara"]);
        assert_eq!(cards[0].raw["data"]["extensions"]["perchance_data"]["avatar_url"], "https://example.org/moos.webp");
    }

    #[test]
    fn pygmalion_and_ooba_json() {
        let (format, card) = single(br#"{"char_name":"Moos","char_persona":"Waldgeist","char_greeting":"Hallo.","world_scenario":"Nebelwald"}"#);
        assert_eq!(format, Format::JsonPygmalion);
        assert_eq!((card.card.name.as_str(), card.card.scenario.as_str()), ("Moos", "Nebelwald"));
        round_trip(&card);
        let (format, card) = single(br#"{"name":"Moos","context":"Waldgeist","greeting":"Hallo."}"#);
        assert_eq!(format, Format::JsonOoba);
        assert_eq!(card.card.description, "Waldgeist");
        round_trip(&card);
    }

    #[test]
    fn risu_persona() {
        let file = detect_and_parse(PERSONA).unwrap();
        assert_eq!(file.format, Format::RisuPersonaPng);
        let Content::Persona { persona, image } = file.content else { panic!("not a persona") };
        assert_eq!(persona.name, "Jonas");
        assert!(persona.prompt.contains("Fährmechaniker"));
        assert_eq!(persona.note, "SFW-Beispielpersona");
        let image = image.unwrap();
        let png = write_persona_png(&persona, Some(&image.bytes)).unwrap();
        check_png_crcs(&png, false);
        let again = detect_and_parse(&png).unwrap();
        assert_eq!(again.format, Format::RisuPersonaPng);
        let Content::Persona { persona: back, .. } = again.content else { panic!() };
        assert_eq!(back, persona);
        assert_eq!(write_persona_png(&back, Some(&png)).unwrap(), png);
        // Without a picture: grey placeholder.
        let plain = write_persona_png(&persona, None).unwrap();
        assert!(matches!(detect_and_parse(&plain).unwrap().content, Content::Persona { .. }));
    }

    #[test]
    fn placeholder_is_a_valid_png() {
        let png = placeholder_png();
        check_png_crcs(&png, false);
        let idat = &png[8 + 25 + 8..png.len() - 12 - 4];
        let mut rows = vec![];
        flate2::read::ZlibDecoder::new(idat).read_to_end(&mut rows).unwrap();
        assert_eq!(rows.len(), 257 * 256);
        let (_, card) = single(&write_png(&json!({"name": "Moos"}), None).unwrap());
        assert_eq!(card.card.name, "Moos");
    }

    #[test]
    fn errors() {
        assert_eq!(detect_and_parse(b"hello").unwrap_err(), ImportError::UnknownFormat);
        assert_eq!(detect_and_parse(&[0u8, 159, 146, 150]).unwrap_err(), ImportError::UnknownFormat);
        assert_eq!(detect_and_parse(b"{ broken").unwrap_err(), ImportError::Broken);
        assert_eq!(detect_and_parse(b"{\"description\":\"no name\"}").unwrap_err(), ImportError::NoCard);
        assert_eq!(detect_and_parse(&placeholder_png()).unwrap_err(), ImportError::NoCard);
        assert_eq!(detect_and_parse(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap_err(), ImportError::NoCard);
        assert_eq!(detect_and_parse(&CHARX[..CHARX.len() / 2]).unwrap_err(), ImportError::Broken);
        assert_eq!(detect_and_parse(&PERCHANCE[..PERCHANCE.len() - 20]).unwrap_err(), ImportError::Broken);
        assert_eq!(detect_and_parse(&vec![0u8; MAX_INPUT + 1]).unwrap_err(), ImportError::TooLarge);
        assert_eq!(write_png(&json!({}), Some(b"GIF89a")).unwrap_err(), WriteError::ImageNotPng);
        assert_eq!(write_png(&json!({}), Some(&ILSE_PNG[..100])).unwrap_err(), WriteError::BrokenImage);

        // A zip with neither card.json nor manifest.json, and one whose card.json is damaged.
        assert_eq!(detect_and_parse(&zip_of(&[("readme.txt", b"hi")])).unwrap_err(), ImportError::NoCard);
        assert_eq!(detect_and_parse(&zip_of(&[("card.json", b"{nope")])).unwrap_err(), ImportError::Broken);
        // Damaged base64 in a PNG card chunk.
        let mut bad = card::png_with_chunk("chara", "{}");
        let at = bad.len() - 18;
        bad[at] = b'!';
        assert_eq!(detect_and_parse(&bad).unwrap_err(), ImportError::Broken);
        // Truncated inputs never panic.
        for bytes in [ILSE_PNG, CHARX, BYAF, PERCHANCE, PERSONA] {
            for cut in (0..bytes.len()).step_by(7) {
                let _ = detect_and_parse(&bytes[..cut]);
            }
        }
    }

    #[test]
    fn bombs_are_stopped_while_inflating() {
        // 4 MiB of spaces deflate to a few KB; with a 1 MiB limit the read stops early.
        let big = vec![b' '; 4 << 20];
        let bytes = zip_of(&[("card.json", &big)]);
        assert!(bytes.len() < 64 << 10);
        let mut zip = Zip::new(&bytes).unwrap();
        assert_eq!(read_entry(&mut zip, "card.json", 1 << 20).unwrap_err(), ImportError::TooLarge);
        assert_eq!(read_entry(&mut zip, "missing", 1 << 20).unwrap(), None);
        // Over the ratio even within the size limit.
        assert_eq!(read_entry(&mut zip, "card.json", 8 << 20).unwrap_err(), ImportError::TooLarge);

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&big).unwrap();
        assert_eq!(gunzip(&gz.finish().unwrap(), 1 << 20).unwrap_err(), ImportError::TooLarge);
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    // ---- Private cards, read locally only. Paths come from the environment, never from the repo;
    // ---- card contents are never printed.

    #[test]
    #[ignore = "needs RP_TEST_CARD_FILE"]
    fn private_card_file() {
        let path = std::env::var("RP_TEST_CARD_FILE").expect("set RP_TEST_CARD_FILE to one card PNG");
        let bytes = std::fs::read(&path).expect("read RP_TEST_CARD_FILE");
        let file = detect_and_parse(&bytes).unwrap_or_else(|e| panic!("not readable: {}", e.code()));
        assert!(matches!(file.format, Format::PngChara | Format::PngCcv3), "format {}", file.format.code());
        let Content::Cards(cards) = &file.content else { panic!("not a card") };
        assert_eq!(cards.len(), 1);
        assert!(!cards[0].card.name.is_empty());
        let png = write_png(&cards[0].raw, Some(&bytes)).expect("write back");
        assert_eq!(single(&png).1.card, cards[0].card);
        println!("format: {}, round trip ok", file.format.code());
    }

    #[test]
    #[ignore = "needs RP_TEST_CARDS_DIR"]
    fn private_cards_dir() {
        let root = std::env::var("RP_TEST_CARDS_DIR").expect("set RP_TEST_CARDS_DIR to a folder of cards");
        let mut files = vec![];
        let mut dirs = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("read dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| ["png", "json", "charx", "yaml", "yml"].contains(&e.to_ascii_lowercase().as_str()))
                {
                    files.push(path);
                }
            }
        }
        let (mut read, mut no_card, mut errors) = (0, 0, vec![]);
        let mut formats = std::collections::BTreeMap::<&str, usize>::new();
        for path in &files {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            match std::fs::read(path).map_err(|_| "io").and_then(|b| detect_and_parse(&b).map_err(|e| e.code())) {
                Ok(file) => {
                    read += 1;
                    *formats.entry(file.format.code()).or_default() += 1;
                }
                Err("no_card" | "unknown_format") => no_card += 1,
                Err(code) => errors.push(format!("{name}: {code}")),
            }
        }
        println!("files: {}, read: {read}, no card: {no_card}, errors: {}", files.len(), errors.len());
        println!("formats: {formats:?}");
        for e in &errors {
            println!("error: {e}");
        }
        assert!(errors.is_empty());
    }
}
