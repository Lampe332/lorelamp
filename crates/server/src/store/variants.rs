//! Reply variants the model writes in the app ("Neu generieren") and replies it goes on writing
//! ("Weiter schreiben"). They use the columns of migration 17: `messages.swipes` (JSON list) and
//! `messages.swipe_id`; `content` is always the active variant.

use rusqlite::{params, OptionalExtension};

use super::{now, Res, Store, Timings};

impl Store {
    /// Adds `text` as a new variant of a reply and makes it the active one; a reply without
    /// variants keeps its present text as the first. Answers the new variant's index, None: no
    /// such message.
    pub fn add_variant(&mut self, id: i64, text: &str, timings: Option<&Timings>) -> Res<Option<usize>> {
        let present: Option<String> = self.conn.query_row("SELECT content FROM messages WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        let Some(present) = present else { return Ok(None) };
        let mut swipes = self.message_swipes(id)?.map_or_else(|| vec![present], |(s, _)| s);
        swipes.push(text.to_string());
        let index = swipes.len() - 1;
        let timings = timings.map(serde_json::to_string).transpose()?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE messages SET content = ?2, swipes = ?3, swipe_id = ?4, timings = ?5 WHERE id = ?1",
            params![id, text, serde_json::to_string(&swipes)?, index as i64, timings],
        )?;
        tx.execute("UPDATE chats SET updated_at = ?2 WHERE id = (SELECT chat_id FROM messages WHERE id = ?1)", params![id, now()])?;
        tx.commit()?;
        Ok(Some(index))
    }

    /// The reply went on: `content` is its whole text now (with variants, the active one changes
    /// with it), `timings` those of the request that added to it.
    pub fn continue_message(&mut self, id: i64, content: &str, timings: Option<&Timings>) -> Res<()> {
        self.edit_message(id, content)?;
        if let Some(t) = timings {
            self.conn.execute("UPDATE messages SET timings = ?2 WHERE id = ?1", params![id, serde_json::to_string(t)?])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{sample, temp_store};
    use super::*;

    #[test]
    fn a_new_variant_keeps_the_old_text_and_a_continued_reply_its_variants() {
        let (mut s, dir) = temp_store();
        let character = s.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let chat = s.create_chat(character, None).unwrap();
        s.add_message(chat, "user", "Wohin?").unwrap();
        let reply = s.add_message(chat, "assistant", "Nach Amrum.").unwrap().id;
        let t = Timings { tok_s: 9.5, processed: 3, cached: 40, predicted: 7 };

        assert_eq!(s.add_variant(reply, "Nach Föhr.", Some(&t)).unwrap(), Some(1));
        assert_eq!(s.add_variant(reply, "Kaffee.", None).unwrap(), Some(2));
        assert_eq!(s.add_variant(reply + 100, "x", None).unwrap(), None);
        let m = s.messages(chat).unwrap().pop().unwrap();
        assert_eq!((m.content.as_str(), m.swipe_id, m.timings.clone()), ("Kaffee.", Some(2), None));
        assert_eq!(m.swipes, Some(vec!["Nach Amrum.".to_string(), "Nach Föhr.".into(), "Kaffee.".into()]));

        // Going on changes the active variant only.
        s.set_swipe(reply, 1, "Nach Föhr.").unwrap();
        s.continue_message(reply, "Nach Föhr. Die Fähre geht um neun.", Some(&t)).unwrap();
        let m = s.messages(chat).unwrap().pop().unwrap();
        assert_eq!((m.content.as_str(), m.swipe_id, m.timings), ("Nach Föhr. Die Fähre geht um neun.", Some(1), Some(t)));
        assert_eq!(m.swipes.unwrap(), ["Nach Amrum.", "Nach Föhr. Die Fähre geht um neun.", "Kaffee."]);
        drop(s);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
