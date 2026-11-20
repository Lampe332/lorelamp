//! SillyTavern chat files (`data/<user>/chats/<character>/<name>.jsonl`): the first line is a head
//! (`user_name`, `character_name`, `create_date`, `chat_metadata`), every further line one message
//! (`name`, `is_user`, `send_date`, `mes`, optional `swipes` + `swipe_id`). Lines with `is_system`
//! (hidden from SillyTavern's prompt, no part of the story) are left out and counted. Pure
//! parsing, no I/O.
//!
//! Dates come in the formats SillyTavern has written over the years: Unix milliseconds, ISO 8601
//! (`2026-03-01T18:31:05.000Z`), `humanizedDateTime` (`2026-03-01@18h30m12s`, local time) and
//! `getMessageTimeStamp` (`March 1, 2026 6:30pm`, local time). Local times are read with the
//! asking device's offset `tz` (minutes east of UTC).

use serde_json::Value;

/// One message of a chat.
#[derive(Debug, Clone, PartialEq)]
pub struct StMessage {
    pub is_user: bool,
    pub name: String,
    /// Unix seconds; None when the line has no readable date.
    pub sent: Option<i64>,
    /// The text SillyTavern shows (`mes`, the chosen variant).
    pub text: String,
    /// All variants of a reply, `text` among them at `swipe_id`; empty when there is only one.
    pub swipes: Vec<String>,
    pub swipe_id: usize,
}

/// A whole chat file.
#[derive(Debug, Clone, PartialEq)]
pub struct StChat {
    pub user_name: String,
    pub character_name: String,
    /// Unix seconds from the head's `create_date`.
    pub created: Option<i64>,
    pub messages: Vec<StMessage>,
    /// Lines SillyTavern marks `is_system` (hidden from its prompt, no part of the story): not in
    /// `messages`, only counted.
    pub system_skipped: usize,
}

/// The non-empty lines of a chat file after the head (what makes a chat the same chat: the head
/// changes whenever SillyTavern saves its metadata).
pub fn message_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty()).peekable();
    let head = lines.peek().and_then(|l| serde_json::from_str::<Value>(l).ok()).is_some_and(|v| is_head(&v));
    lines.skip(usize::from(head))
}

fn is_head(v: &Value) -> bool {
    v.is_object() && v.get("mes").is_none() && (v.get("chat_metadata").is_some() || v.get("character_name").is_some() || v.get("user_name").is_some())
}

/// Reads a chat file. Err `broken`: not UTF-8, or a line that is no JSON object.
pub fn parse(bytes: &[u8], tz: i32) -> Result<StChat, &'static str> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).map_err(|_| "broken")?;
    let first = text.lines().map(str::trim).find(|l| !l.is_empty());
    let head = first.and_then(|l| serde_json::from_str::<Value>(l).ok()).filter(is_head).unwrap_or(Value::Null);
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let mut chat = StChat {
        user_name: s(&head["user_name"]),
        character_name: s(&head["character_name"]),
        created: parse_date(&head["create_date"], tz),
        messages: vec![],
        system_skipped: 0,
    };
    for line in message_lines(text) {
        let v: Value = serde_json::from_str(line).map_err(|_| "broken")?;
        if !v.is_object() {
            return Err("broken");
        }
        if v["is_system"].as_bool() == Some(true) {
            chat.system_skipped += 1;
            continue;
        }
        let text = s(&v["mes"]);
        let mut swipes: Vec<String> = v["swipes"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let mut swipe_id = v["swipe_id"].as_u64().map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
        if swipes.len() < 2 {
            swipes.clear();
            swipe_id = 0;
        } else {
            swipe_id = swipe_id.min(swipes.len() - 1);
            // `mes` is what SillyTavern shows (an edit changes only `mes`).
            swipes[swipe_id].clone_from(&text);
        }
        chat.messages.push(StMessage { is_user: v["is_user"].as_bool().unwrap_or(false), name: s(&v["name"]), sent: parse_date(&v["send_date"], tz), text, swipes, swipe_id });
    }
    Ok(chat)
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn seconds(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    let ok = (1..=12).contains(&mo) && (1..=31).contains(&d) && (0..=23).contains(&h) && (0..=59).contains(&mi) && (0..=60).contains(&s);
    ok.then(|| days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s)
}

/// A date in any of SillyTavern's formats as Unix seconds (see the module docs).
pub fn parse_date(v: &Value, tz: i32) -> Option<i64> {
    if let Some(n) = v.as_f64() {
        return from_number(n);
    }
    let t = v.as_str()?.trim();
    if t.is_empty() {
        return None;
    }
    if t.bytes().all(|b| b.is_ascii_digit()) {
        return from_number(t.parse::<f64>().ok()?);
    }
    let local = i64::from(tz) * 60;
    iso(t, local).or_else(|| humanized(t).map(|s| s - local)).or_else(|| spoken(t).map(|s| s - local))
}

/// Milliseconds (as JavaScript's `Date.now()`) or, for small numbers, seconds.
fn from_number(n: f64) -> Option<i64> {
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    Some(if n >= 1e11 { (n / 1000.0) as i64 } else { n as i64 })
}

fn num(t: &str) -> Option<i64> {
    (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit())).then(|| t.parse().ok()).flatten()
}

/// `2026-03-01T18:31:05.000Z`, `…+02:00`, or without zone (then local).
fn iso(t: &str, local: i64) -> Option<i64> {
    let (date, time) = t.split_once(['T', ' '])?;
    let mut d = date.split('-');
    let (y, mo, da) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let (clock, offset) = if let Some(c) = time.strip_suffix('Z') {
        (c, 0)
    } else if let Some(at) = time.rfind(['+', '-']).filter(|&i| i >= 5) {
        let sign = if time.as_bytes()[at] == b'-' { -1 } else { 1 };
        let (oh, om) = time[at + 1..].split_once(':').unwrap_or((&time[at + 1..], "0"));
        (&time[..at], sign * (num(oh)? * 3600 + num(om)? * 60))
    } else {
        (time, local)
    };
    let clock = clock.split('.').next()?;
    let mut c = clock.split(':');
    let (h, mi) = (num(c.next()?)?, num(c.next()?)?);
    let s = c.next().map_or(Some(0), num)?;
    Some(seconds(y, mo, da, h, mi, s)? - offset)
}

/// `2026-03-01@18h30m12s` with optional `123ms` (local time).
fn humanized(t: &str) -> Option<i64> {
    let (date, time) = t.split_once('@')?;
    let mut d = date.split('-');
    let (y, mo, da) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let (h, rest) = time.split_once('h')?;
    let (mi, rest) = rest.split_once('m')?;
    let s = rest.split_once('s').map_or("0", |(s, _)| s);
    seconds(y, mo, da, num(h)?, num(mi)?, num(s)?)
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// `March 1, 2026 6:30pm` / `Mar 1, 2026 6:30 PM` / `March 1, 2026 18:30` (local time).
fn spoken(t: &str) -> Option<i64> {
    let lower = t.to_ascii_lowercase().replace(',', " ");
    let mut words = lower.split_whitespace();
    let month = words.next()?;
    let mo = MONTHS.iter().position(|m| month.len() >= 3 && month.starts_with(m))? as i64 + 1;
    let (d, y) = (num(words.next()?)?, num(words.next()?)?);
    let mut clock: String = words.collect();
    let pm = clock.ends_with("pm");
    let am = clock.ends_with("am");
    if pm || am {
        clock.truncate(clock.len() - 2);
    }
    let (h, mi) = clock.split_once(':')?;
    let (mut h, mi) = (num(h)?, num(mi)?);
    if am || pm {
        if !(1..=12).contains(&h) {
            return None;
        }
        h = h % 12 + if pm { 12 } else { 0 };
    }
    seconds(y, mo, d, h, mi, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 2026-03-01 18:30:00 UTC.
    const T: i64 = 1_772_389_800;

    #[test]
    fn dates_in_every_format() {
        assert_eq!(parse_date(&json!(1_772_389_800_000_i64), 0), Some(T));
        assert_eq!(parse_date(&json!("1772389800000"), 0), Some(T));
        assert_eq!(parse_date(&json!(T), 0), Some(T));
        assert_eq!(parse_date(&json!("2026-03-01T18:30:00.000Z"), 60), Some(T));
        assert_eq!(parse_date(&json!("2026-03-01T19:30:00+01:00"), 0), Some(T));
        assert_eq!(parse_date(&json!("2026-03-01T13:30:00-05:00"), 0), Some(T));
        assert_eq!(parse_date(&json!("2026-03-01T19:30:00"), 60), Some(T), "no zone: local");
        assert_eq!(parse_date(&json!("2026-03-01@19h30m00s"), 60), Some(T));
        assert_eq!(parse_date(&json!("2026-03-01@18h30m00s123ms"), 0), Some(T));
        assert_eq!(parse_date(&json!("March 1, 2026 7:30pm"), 60), Some(T));
        assert_eq!(parse_date(&json!("Mar 1, 2026 6:30 PM"), 0), Some(T));
        assert_eq!(parse_date(&json!("March 1, 2026 18:30"), 0), Some(T));
        assert_eq!(parse_date(&json!("March 1, 2026 12:05am"), 0), Some(T - 18 * 3600 - 25 * 60));
        assert_eq!(parse_date(&json!("March 1, 2026 12:05pm"), 0), Some(T - 6 * 3600 - 25 * 60));
        for bad in [json!(""), json!("gestern"), json!("2026-13-01T00:00:00Z"), json!(null), json!(-5), json!("March 1, 2026 13:00pm")] {
            assert_eq!(parse_date(&bad, 0), None, "{bad}");
        }
    }

    #[test]
    fn head_messages_and_swipes() {
        let file = [
            json!({ "user_name": "Jonas", "character_name": "Theo", "create_date": "2026-03-01@18h30m00s", "chat_metadata": {} }),
            json!({ "name": "Theo", "is_user": false, "send_date": "March 1, 2026 6:30pm", "mes": "Moin.", "swipes": ["Hallo.", "Moin."], "swipe_id": 1 }),
            json!({ "name": "Jonas", "is_user": true, "send_date": "2026-03-01T18:31:00.000Z", "mes": "Hallo Theo." }),
            json!({ "name": "Theo", "is_user": false, "send_date": 1_772_389_920_000_i64, "mes": "Bearbeitet", "swipes": ["a", "b", "c"], "swipe_id": 9 }),
            json!({ "name": "Theo", "is_user": false, "mes": "Allein", "swipes": ["Allein"], "swipe_id": 0 }),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let chat = parse(format!("\u{feff}{file}\n\n").as_bytes(), 0).unwrap();
        assert_eq!((chat.user_name.as_str(), chat.character_name.as_str(), chat.created), ("Jonas", "Theo", Some(T)));
        let m = &chat.messages;
        assert_eq!(m.len(), 4);
        assert_eq!((m[0].is_user, m[0].sent, m[0].text.as_str(), m[0].swipes.clone(), m[0].swipe_id), (false, Some(T), "Moin.", vec!["Hallo.".to_string(), "Moin.".into()], 1));
        assert_eq!((m[1].is_user, m[1].name.as_str(), m[1].sent, m[1].swipes.len()), (true, "Jonas", Some(T + 60), 0));
        assert_eq!((m[2].sent, m[2].swipe_id, m[2].swipes.clone()), (Some(T + 120), 2, vec!["a".to_string(), "b".into(), "Bearbeitet".into()]), "id clamped, mes wins");
        assert_eq!((m[3].sent, m[3].swipes.len()), (None, 0), "one variant is no swipe");
        assert_eq!(message_lines(&file).count(), 4);

        // Without a head every line is a message.
        let bare = json!({ "name": "Theo", "is_user": false, "mes": "Hi" }).to_string();
        let chat = parse(bare.as_bytes(), 0).unwrap();
        assert_eq!((chat.messages.len(), chat.character_name.as_str()), (1, ""));
        assert_eq!(message_lines(&bare).count(), 1);

        for bad in [&b"\xff\xfe"[..], b"{\"user_name\":\"x\",\"chat_metadata\":{}}\nkein json", b"{\"chat_metadata\":{}}\n[1,2]"] {
            assert_eq!(parse(bad, 0), Err("broken"));
        }
        assert_eq!(parse(b"", 0).unwrap().messages.len(), 0);
    }

    /// Lines SillyTavern marks `is_system` (hidden from its prompt) are no messages; they are
    /// counted.
    #[test]
    fn system_lines_are_left_out() {
        let file = [
            json!({ "user_name": "Jonas", "character_name": "Theo", "chat_metadata": {} }),
            json!({ "name": "Theo", "is_user": false, "is_system": false, "mes": "Moin." }),
            json!({ "name": "System", "is_user": false, "is_system": true, "mes": "Versteckt." }),
            json!({ "name": "Jonas", "is_user": true, "mes": "Hallo." }),
            json!({ "name": "Jonas", "is_user": true, "is_system": true, "mes": "Ausgeblendet." }),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let chat = parse(file.as_bytes(), 0).unwrap();
        assert_eq!(chat.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Moin.", "Hallo."]);
        assert_eq!(chat.system_skipped, 2);
        assert_eq!(message_lines(&file).count(), 4, "the hash still covers every line");
    }
}
