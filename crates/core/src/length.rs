//! The answer length of a model profile (`n_predict`) as a real length, never exceeded:
//! - the prompt tells the model how long to write ([`crate::prompt`], per-turn note),
//! - from [`MARK_PERCENT`] of the length on, the reply ends at the first *clean* sentence end:
//!   one where no speech (quotation marks) and no *action* or _thought_ is still open
//!   ([`SoftStop`]),
//! - the model server gets the length itself as its hard cap ([`hard_cap`]); a reply that reaches
//!   it (or ends otherwise without a clean end) is cut back to the last sentence end since the
//!   mark and the marks still open are closed ([`SoftStop::finish`]),
//! - a profile with reasoning keeps the old, wider cap ([`cap`]): its thinking counts into the
//!   same tokens and must not be ended mid-way.

/// Above this many tokens the answer length is only an upper bound: no length note in the prompt
/// (nobody asks a character for 1,500 words; the setting then just keeps runaway replies short).
pub const NOTE_MAX_TOKENS: i64 = 2048;

/// From this share of the length on (in percent) the reply looks for a clean sentence end. A
/// sentence runs 20 to 40 tokens, so the last 15 % (60 tokens of 400) nearly always hold one,
/// while the replies still use almost all of the length that was set. Lower: shorter replies than
/// asked for; higher: too little room to find a clean end before the hard cap.
pub const MARK_PERCENT: u64 = 85;

/// The most tokens the model server may write for a reply that aims at `target` tokens when its
/// thinking shares them (reasoning): half as much again, but at most 200 more.
pub fn cap(target: i64) -> i64 {
    let target = target.max(1);
    target.saturating_add((target / 2).min(200))
}

/// The most tokens the model server may write for a reply: the answer length itself, so no reply
/// is ever longer. With reasoning on: [`cap`].
pub fn hard_cap(target: i64, reasoning: bool) -> i64 {
    if reasoning {
        cap(target)
    } else {
        target.max(1)
    }
}

/// Characters that end a sentence.
fn terminator(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？')
}

/// Characters that may close a sentence after its end: quotes, the asterisks and underscores of
/// actions and thoughts in italics, backticks of thoughts, brackets.
fn closer(c: char) -> bool {
    matches!(c, '"' | '\'' | '”' | '’' | '“' | '»' | '«' | '*' | '_' | '~' | '`' | ')' | ']')
}

/// Words before a full stop that is no sentence end (lower case).
const ABBREVIATIONS: [&str; 12] = ["mr", "mrs", "ms", "dr", "prof", "st", "nr", "bzw", "ca", "vgl", "jr", "sr"];

/// Whether a full stop at byte `dot` of `text` ends a sentence by what stands before it: not after
/// a single letter ("z. B."), a number ("am 3. Mai") or a known abbreviation ("Mr.").
fn full_stop_ends(text: &str, dot: usize) -> bool {
    let before = &text[..dot];
    let word: String = before.chars().rev().take_while(|c| c.is_alphanumeric()).collect();
    if word.is_empty() {
        return true;
    }
    let word: String = word.chars().rev().collect::<String>().to_lowercase();
    !(word.chars().count() == 1 || word.chars().all(|c| c.is_ascii_digit()) || ABBREVIATIONS.contains(&word.as_str()))
}

/// Something a reply has opened and not closed yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Open {
    /// „ … “ (de, ru)
    DeLow,
    /// “ … ” (en, es)
    Curly,
    /// « … » (ru, es)
    GuilLeft,
    /// » … « (de)
    GuilRight,
    /// " … "
    Straight,
    /// *action*
    Italic,
    /// **strong**
    Bold,
    /// _thought_
    Under,
    /// `thought`
    Tick,
}

impl Open {
    /// What closes it.
    fn closing(self) -> &'static str {
        match self {
            Open::DeLow => "“",
            Open::Curly => "”",
            Open::GuilLeft => "»",
            Open::GuilRight => "«",
            Open::Straight => "\"",
            Open::Italic => "*",
            Open::Bold => "**",
            Open::Under => "_",
            Open::Tick => "`",
        }
    }
}

/// The marks that close everything in `open`, innermost first.
fn closing(open: &[Open]) -> String {
    open.iter().rev().map(|o| o.closing()).collect()
}

/// A place where a sentence ends and what is still open there.
#[derive(Debug, Clone, PartialEq, Eq)]
struct End {
    /// The byte where the reply may stop (after the end mark and its closing quotes or asterisks;
    /// before a line break).
    at: usize,
    open: Vec<Open>,
}

/// Reads a text char by char and keeps what is open. Every kind of mark closes only its own
/// opener, whatever else is open stays open (a reply that leaves an action open inside a quote
/// is reported as open until it closes it). Not tracked: single quotes (apostrophes make them
/// unreliable) and brackets.
struct Scan<'a> {
    cs: &'a [(usize, char)],
    stack: Vec<Open>,
}

impl Scan<'_> {
    /// Closes the last `o` that is open; false if none is.
    fn close(&mut self, o: Open) -> bool {
        match self.stack.iter().rposition(|&x| x == o) {
            Some(i) => {
                self.stack.remove(i);
                true
            }
            None => false,
        }
    }

    /// An emphasis mark (symmetric): closes its opener if open, else opens it. As an opener it
    /// needs a visible character after it and no letter before it, and a mark between two
    /// letters opens or closes nothing (snake_case, "Student*innen"), so "3 * 4" and the like
    /// stay out.
    fn emphasis(&mut self, o: Open, prev: Option<char>, next: Option<char>) {
        let inside = prev.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric);
        if inside {
            return;
        }
        if !self.close(o) && next.is_some_and(|n| !n.is_whitespace()) && !prev.is_some_and(char::is_alphanumeric) {
            self.stack.push(o);
        }
    }

    /// Takes the char at index `k` into account; returns how many chars it consumed (a run of
    /// asterisks or underscores is one mark).
    fn mark(&mut self, k: usize) -> usize {
        let c = self.cs[k].1;
        let prev = k.checked_sub(1).map(|p| self.cs[p].1);
        let run = self.cs[k..].iter().take_while(|&&(_, d)| d == c).count();
        let next = self.cs.get(k + run).map(|&(_, d)| d);
        match c {
            '„' => self.stack.push(Open::DeLow),
            '“' => {
                if !self.close(Open::DeLow) {
                    self.stack.push(Open::Curly);
                }
            }
            '”' => {
                if !self.close(Open::Curly) {
                    self.close(Open::DeLow);
                }
            }
            '«' => {
                if !self.close(Open::GuilRight) {
                    self.stack.push(Open::GuilLeft);
                }
            }
            '»' => {
                if !self.close(Open::GuilLeft) {
                    self.stack.push(Open::GuilRight);
                }
            }
            '"' => {
                // A straight quote also closes a German „ (models mix the styles).
                if !self.close(Open::Straight) && !self.close(Open::DeLow) {
                    self.stack.push(Open::Straight);
                }
            }
            '*' => {
                match run {
                    1 => self.emphasis(Open::Italic, prev, next),
                    2 => self.emphasis(Open::Bold, prev, next),
                    _ => {
                        self.emphasis(Open::Bold, prev, next);
                        self.emphasis(Open::Italic, prev, next);
                    }
                }
                return run;
            }
            '_' => {
                // Only a single underscore at the edge of a word is an italic mark.
                if run == 1 {
                    self.emphasis(Open::Under, prev, next);
                }
                return run;
            }
            '`' => {
                self.emphasis(Open::Tick, prev, next);
                return run;
            }
            _ => {}
        }
        1
    }
}

/// The sentence ends of `text` at or after byte `from`, with what is open at each (`seed`: what
/// is open before the text starts), and what is open at the end of the text. An end mark only
/// counts once the next character is known and is white space, so "3.5", "Hm..." in the middle of
/// a word or a closing quote still to come end nothing; `eof`: the text is complete, so an end
/// mark at its very end counts. A line break ends a sentence by itself (the cut is before it).
fn scan(text: &str, seed: &[Open], from: usize, eof: bool) -> (Vec<End>, Vec<Open>) {
    let cs: Vec<(usize, char)> = text.char_indices().collect();
    let mut s = Scan { cs: &cs, stack: seed.to_vec() };
    let mut ends: Vec<End> = vec![];
    let mut i = 0;
    while i < cs.len() {
        let (pos, c) = cs[i];
        if c == '\n' {
            if pos >= from && ends.last().is_none_or(|e| e.at != pos) {
                ends.push(End { at: pos, open: s.stack.clone() });
            }
            i += 1;
            continue;
        }
        if !terminator(c) {
            i += s.mark(i);
            continue;
        }
        // "?!", "..." – one end; a run with more than a full stop is a real end.
        let mut stops = c != '.' || full_stop_ends(text, pos);
        let mut k = i + 1;
        while let Some(&(_, d)) = cs.get(k) {
            if terminator(d) {
                stops |= d != '.';
                k += 1;
            } else if closer(d) {
                k += s.mark(k);
            } else {
                break;
            }
        }
        let at = cs.get(k).map_or(text.len(), |&(j, _)| j);
        let known = match cs.get(k) {
            Some(&(_, next)) => next.is_whitespace(),
            None => eof,
        };
        if known && stops && at >= from {
            ends.push(End { at, open: s.stack.clone() });
        }
        i = k;
    }
    (ends, s.stack)
}

/// What is open at byte `upto` of `text` (after `seed`).
fn open_at(text: &str, seed: &[Open], upto: usize) -> Vec<Open> {
    scan(&text[..upto], seed, usize::MAX, true).1
}

/// What to do with a piece of the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Hand this text on and read further.
    Go(String),
    /// Hand this text on, then end the reply: it reached its length and a clean sentence ended.
    End(String),
}

/// Follows the text of one reply (only the part written now; a reply written on starts empty,
/// see [`SoftStop::with_base`]) and ends it at the first clean sentence end once it has
/// [`MARK_PERCENT`] of `target` tokens. Until then every piece goes on as it comes. After it,
/// only text up to the last sentence end goes on and the running sentence waits, so no device
/// ever sees text that the reply then loses: everything handed on stays in the reply, and
/// [`SoftStop::finish`] only cuts what was held back (and adds closing marks).
#[derive(Debug, Clone)]
pub struct SoftStop {
    /// Tokens from which clean sentence ends count.
    mark: u64,
    /// What is open before the text starts (the text a reply is written on from).
    seed: Vec<Open>,
    text: String,
    /// Bytes of `text` handed on.
    sent: usize,
    /// Where sentence ends count (set once the mark is reached).
    from: Option<usize>,
}

impl SoftStop {
    pub fn new(target: i64) -> SoftStop {
        let target = u64::try_from(target.max(1)).unwrap_or(1);
        SoftStop { mark: (target.saturating_mul(MARK_PERCENT) / 100).max(1), seed: vec![], text: String::new(), sent: 0, from: None }
    }

    /// The reply is written on from `base` (the text the message holds already): a quote or an
    /// action that `base` left open (a reply that was cut off) is still open for the new text,
    /// so it does not end inside it. A clean base (every reply this module ends) changes nothing.
    pub fn with_base(mut self, base: &str) -> SoftStop {
        self.seed = scan(base, &[], usize::MAX, true).1;
        self
    }

    /// `piece` arrived and the reply has `tokens` tokens now.
    pub fn push(&mut self, piece: &str, tokens: u64) -> Step {
        let before = self.text.len();
        self.text.push_str(piece);
        if self.from.is_none() && tokens >= self.mark {
            // Ends count from the piece that reached the mark on.
            self.from = Some(before);
        }
        let Some(from) = self.from else { return Step::Go(self.take(self.text.len())) };
        let (ends, _) = scan(&self.text, &self.seed, from, false);
        if let Some(e) = ends.iter().find(|e| e.open.is_empty()) {
            return Step::End(self.take(e.at));
        }
        // No clean end yet: what stands before the last end (clean or not) goes on, the running
        // sentence waits for its end.
        let upto = ends.last().map_or(self.sent, |e| e.at);
        Step::Go(self.take(upto))
    }

    /// The stream ended (by itself or at the hard cap): what of the held text still goes on.
    /// Before the mark: all of it. After it: the text up to a clean sentence end, else up to the
    /// last sentence end since the mark with the marks still open there closed, else nothing
    /// (the running sentence is dropped) and the marks open at the end of what was handed on
    /// closed.
    pub fn finish(&mut self) -> String {
        let Some(from) = self.from else { return self.take(self.text.len()) };
        let (ends, _) = scan(&self.text, &self.seed, from, true);
        if let Some(e) = ends.iter().find(|e| e.open.is_empty()) {
            return self.take(e.at);
        }
        let upto = ends.last().map_or(self.sent, |e| e.at).max(self.sent);
        let mut out = self.take(upto);
        out.push_str(&closing(&open_at(&self.text, &self.seed, self.sent)));
        out
    }

    fn take(&mut self, upto: usize) -> String {
        let upto = upto.max(self.sent);
        let out = self.text[self.sent..upto].to_string();
        self.sent = upto;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hard_cap_is_the_length_unless_the_model_thinks() {
        assert_eq!(hard_cap(400, false), 400);
        assert_eq!(hard_cap(1, false), 1);
        assert_eq!(hard_cap(0, false), 1);
        assert_eq!(hard_cap(i64::MAX, false), i64::MAX);
        assert_eq!(hard_cap(400, true), 600, "reasoning keeps the wider cap");
        assert_eq!(hard_cap(150, true), cap(150));
        assert_eq!(cap(150), 225, "half as much again");
        assert_eq!(cap(400), 600);
        assert_eq!(cap(800), 1000, "at most 200 more");
        assert_eq!(cap(1), 1);
        assert_eq!(cap(0), 1);
        assert_eq!(cap(i64::MAX), i64::MAX);
    }

    #[test]
    fn the_mark_is_85_percent_of_the_length() {
        let mark = |t| SoftStop::new(t).mark;
        assert_eq!((mark(400), mark(800), mark(150), mark(10), mark(1), mark(0)), (340, 680, 127, 8, 1, 1));
    }

    /// The text up to the first sentence end (clean or not) and the first clean one.
    fn first(t: &str) -> (Option<&str>, Option<&str>) {
        let (ends, _) = scan(t, &[], 0, false);
        (ends.first().map(|e| &t[..e.at]), ends.iter().find(|e| e.open.is_empty()).map(|e| &t[..e.at]))
    }

    #[test]
    fn sentence_ends() {
        fn end(t: &str) -> Option<&str> {
            first(t).0
        }
        assert_eq!(end("She nods. Then"), Some("She nods."));
        assert_eq!(end("\"Come in!\" she says"), Some("\"Come in!\""));
        assert_eq!(end("*She smiles.* \"Hi"), Some("*She smiles.*"));
        assert_eq!(end("„Komm rein.“ Sie"), Some("„Komm rein.“"));
        assert_eq!(end("Really?! No"), Some("Really?!"));
        assert_eq!(end("Well… maybe"), Some("Well…"));
        assert_eq!(end("Hm... ok"), Some("Hm..."));
        assert_eq!(end("Ты пришла? Да"), Some("Ты пришла?"));
        assert_eq!(end("no mark at all\nnext"), Some("no mark at all"));
        assert_eq!(end("She nods."), None, "the next character is not known yet");
        assert_eq!(end("She nods.\""), None, "a closing quote may follow");
        assert_eq!(end("It costs 3.5 coins"), None);
        assert_eq!(end("Mr. Smith waits"), None);
        assert_eq!(end("Das ist z. B. gut"), None);
        assert_eq!(end("Am 3. Mai kam sie"), None);
        assert_eq!(end("Mr.? Hm"), Some("Mr.?"), "a question mark ends it anyway");
        assert_eq!(scan("One. Two. Three", &[], 5, false).0.first().map(|e| e.at), Some(9));
        assert_eq!(scan("She nods.", &[], 0, true).0.len(), 1, "at the end of the text the end counts");
        assert_eq!(scan("It costs 3.", &[], 0, true).0.len(), 0, "unless a number stands before it");
    }

    #[test]
    fn a_sentence_end_inside_speech_or_an_action_is_not_clean() {
        let clean = |t| first(t).1;
        // de „…“
        assert_eq!(clean("Er sagt: „Vergiss es. Geh.“ Dann"), Some("Er sagt: „Vergiss es. Geh.“"));
        assert_eq!(clean("Er sagt: „Vergiss es. Geh weg"), None);
        // en “…”
        assert_eq!(clean("He says: “Forget it. Go.” Then"), Some("He says: “Forget it. Go.”"));
        assert_eq!(clean("He says: “Forget it. Go away"), None);
        // es “…” and «…»
        assert_eq!(clean("Dice: «Olvídalo. Vete.» Luego"), Some("Dice: «Olvídalo. Vete.»"));
        assert_eq!(clean("Dice: «Olvídalo. Vete"), None);
        assert_eq!(clean("Dice: “Olvídalo. Vete.” Luego"), Some("Dice: “Olvídalo. Vete.”"));
        // ru «…» and „…“
        assert_eq!(clean("Она сказала: «Уходи. Сейчас.» Потом"), Some("Она сказала: «Уходи. Сейчас.»"));
        assert_eq!(clean("Она сказала: «Уходи. Сейчас"), None);
        assert_eq!(clean("Она сказала: „Уходи. Сейчас.“ Потом"), Some("Она сказала: „Уходи. Сейчас.“"));
        // de »…«
        assert_eq!(clean("Sie sagt: »Geh. Sofort.« Dann"), Some("Sie sagt: »Geh. Sofort.«"));
        assert_eq!(clean("Sie sagt: »Geh. Sofort"), None);
        // straight quotes, also a „ closed by a straight quote
        assert_eq!(clean("She says: \"Forget it. Go.\" Then"), Some("She says: \"Forget it. Go.\""));
        assert_eq!(clean("She says: \"Forget it. Go away"), None);
        assert_eq!(clean("Er sagt: „Vergiss es. Geh.\" Dann"), Some("Er sagt: „Vergiss es. Geh.\""));
        // *action*, **strong**, _thought_, `thought`
        assert_eq!(clean("*Sie brach ab. Dann ging sie.* Danach"), Some("*Sie brach ab. Dann ging sie.*"));
        assert_eq!(clean("*Sie brach ab. Dann ging sie"), None);
        assert_eq!(clean("**Halt. Stopp.** Danach"), Some("**Halt. Stopp.**"));
        assert_eq!(clean("**Halt. Stopp"), None);
        assert_eq!(clean("_Was nun. Wirklich._ Danach"), Some("_Was nun. Wirklich._"));
        assert_eq!(clean("_Was nun. Wirklich"), None);
        assert_eq!(clean("`Was nun. Wirklich.` Danach"), Some("`Was nun. Wirklich.`"));
        // An end after a closed quote and an open action counts only after the action closes.
        assert_eq!(clean("*Sie lächelt. „Ja.“ Dann geht sie.* Weiter"), Some("*Sie lächelt. „Ja.“ Dann geht sie.*"));
        // A quote inside an action closes inside it.
        assert_eq!(clean("*Sie flüstert „Ja.“ und geht.* Weiter"), Some("*Sie flüstert „Ja.“ und geht.*"));
        // Marks that are no markers.
        assert_eq!(clean("Die snake_case Variable. Ende"), Some("Die snake_case Variable."));
        assert_eq!(clean("Liebe Student*innen. Ende"), Some("Liebe Student*innen."));
        assert_eq!(clean("Er rechnet 3 * 4 aus. Ende"), Some("Er rechnet 3 * 4 aus."));
        assert_eq!(clean("Er hat 5' Größe. Ende"), Some("Er hat 5' Größe."), "single quotes are not tracked");
    }

    #[test]
    fn the_marks_to_close() {
        let open = |t: &str| open_at(t, &[], t.len());
        assert_eq!(closing(&open("„Vergiss es.")), "“");
        assert_eq!(closing(&open("He said: “Go")), "”");
        assert_eq!(closing(&open("Dice: «Vete")), "»");
        assert_eq!(closing(&open("Sie sagt: »Geh")), "«");
        assert_eq!(closing(&open("She says \"Go")), "\"");
        assert_eq!(closing(&open("*Sie brach ab.")), "*");
        assert_eq!(closing(&open("**Halt")), "**");
        assert_eq!(closing(&open("_Was nun")), "_");
        assert_eq!(closing(&open("*Sie flüstert „Komm")), "“*", "innermost first");
        assert_eq!(closing(&open("„Komm *her")), "*“");
        assert_eq!(closing(&open("Alles gut. *Ende* „Ja.“")), "");
        // What a base text leaves open.
        let (_, base) = scan("Er sagt: „Vergiss", &[], usize::MAX, true);
        assert_eq!(closing(&base), "“");
        assert_eq!(first("es. Geh.“ Dann").1, Some("es."), "without the base the first end looks clean");
        assert_eq!(scan("es. Geh.“ Dann", &base, 0, false).0.iter().find(|e| e.open.is_empty()).map(|e| e.at), Some(11), "with the base the end counts only after the speech closes");
    }

    /// Feeds `pieces` (one token each) and returns what reached the devices and whether the
    /// reply was ended by a clean sentence end (else the stream ended: by itself or at the hard
    /// cap).
    fn run(target: i64, pieces: &[&str]) -> (String, bool) {
        let mut s = SoftStop::new(target);
        let mut out = String::new();
        for (n, p) in pieces.iter().enumerate() {
            match s.push(p, n as u64 + 1) {
                Step::Go(t) => out.push_str(&t),
                Step::End(t) => {
                    out.push_str(&t);
                    return (out, true);
                }
            }
        }
        out.push_str(&s.finish());
        (out, false)
    }

    #[test]
    fn the_reply_ends_at_the_first_clean_sentence_end_after_the_mark() {
        // Target 10: the mark is token 8 (the second full stop).
        let pieces = ["She", " nods", ".", " Then", " she", " turns", " away", ".", " The", " door", " opens", "."];
        assert_eq!(run(10, &pieces), ("She nods. Then she turns away.".into(), true), "the first end after the mark; the one before does not count");
        assert_eq!(run(9, &pieces), ("She nods. Then she turns away.".into(), true), "mark 7: the mark piece is the one before the end");
        assert_eq!(run(3, &pieces), ("She nods.".into(), true), "mark 2: the first end");
        // The end mark came with the piece that reached the mark; its confirmation follows.
        assert_eq!(run(3, &["\"She", " nods", ".", "\"", " Then"]), ("\"She nods.\"".into(), true));
        // Held-back marks that are no end go on with the text.
        assert_eq!(run(1, &["It", " costs", " 3", ".", "5", " coins", ".", " Done"]), ("It costs 3.5 coins.".into(), true));
        assert_eq!(run(1, &["Mr", ".", " Smith", " waits", ".", "\n", "More"]), ("Mr. Smith waits.".into(), true));
        assert_eq!(run(2, &["*She", " sighs", "*", "\n\n", "\"Hi"]), ("*She sighs*".into(), true), "a line break ends it too");
        // An end exactly at the mark is an end (nothing more is handed on).
        assert_eq!(run(10, &["Ann", " ate", " the", " red", " apple", " in", " bed", ".", " Then", " sleep"]).0, "Ann ate the red apple in bed.");
    }

    #[test]
    fn a_reply_shorter_than_the_mark_stays_as_written() {
        let pieces = ["She", " nods", ".", " Then", " she", " turns", " away", ".", " The", " door"];
        assert_eq!(run(100, &pieces), ("She nods. Then she turns away. The door".into(), false));
        // Even with speech still open at its end: nothing is cut or added.
        assert_eq!(run(100, &["„Vergiss", " es", ".", " Geh"]), ("„Vergiss es. Geh".into(), false));
        assert_eq!(run(100, &["*Sie", " brach", " ab", "."]), ("*Sie brach ab.".into(), false));
    }

    #[test]
    fn a_sentence_end_in_open_speech_does_not_end_the_reply() {
        // Target 10, mark 8: "Dann" is the token that reaches it.
        let pieces = ["Er", " sagt", ":", " „", "Komm", " her", ".", " Dann", " geh", " ich", ".“", " Ende"];
        // "her." ends a sentence before the mark; after it the speech closes at "ich.“".
        assert_eq!(run(10, &pieces), ("Er sagt: „Komm her. Dann geh ich.“".into(), true));
        // Mark 6 (target 8): "her." is after the mark but inside the speech.
        let (out, ended) = run(8, &pieces);
        assert_eq!((out.as_str(), ended), ("Er sagt: „Komm her. Dann geh ich.“", true), "ends where the speech closes");
        // The same with an action.
        let pieces = ["*Sie", " brach", " ab", ".", " Dann", " ging", " sie", ".*", " Weiter"];
        assert_eq!(run(5, &pieces), ("*Sie brach ab. Dann ging sie.*".into(), true));
    }

    #[test]
    fn at_the_hard_cap_the_reply_is_cut_back_to_a_sentence_end_and_closed() {
        // Target 8, mark 6; the stream stops at 8 tokens (the hard cap).
        let cut = |pieces: &[&str]| run(8, pieces);
        // A sentence end after the mark inside speech: cut back to it, close the speech.
        assert_eq!(cut(&["„Komm", " her", " und", " hör", " zu", ".", " Dann", " geh"]), ("„Komm her und hör zu.“".into(), false));
        // "*Sie brach ab." – the asterisk is closed.
        assert_eq!(run(5, &["*Sie", " brach", " ab", ".", " Dann"]), ("*Sie brach ab.*".into(), false));
        // Two open marks close innermost first.
        assert_eq!(cut(&["*Sie", " sagt", " „", "Komm", " zu", " mir", ".", " Dann"]), ("*Sie sagt „Komm zu mir.“*".into(), false));
        // en, ru, es, de »«, straight quotes.
        assert_eq!(cut(&["“", "Come", " here", " and", " listen", " to", " me", ".", " Then"]).0, "“Come here and listen to me.”");
        assert_eq!(cut(&["«", "Иди", " сюда", " и", " слушай", " меня", ".", " Потом"]).0, "«Иди сюда и слушай меня.»");
        assert_eq!(cut(&["«", "Ven", " aquí", " y", " escucha", " mi", ".", " Luego"]).0, "«Ven aquí y escucha mi.»");
        assert_eq!(cut(&["»", "Komm", " her", " und", " hör", " zu", ".", " Dann"]).0, "»Komm her und hör zu.«");
        assert_eq!(cut(&["\"", "Come", " here", " and", " listen", " to", " me", ".", " Then"]).0, "\"Come here and listen to me.\"");
        // The cap hit right after an end mark: the end counts at the end of the stream.
        assert_eq!(cut(&["„Komm", " her", " und", " hör", " gut", " zu", " mir", "."]), ("„Komm her und hör gut zu mir.“".into(), false));
        // A clean end at the end of the stream is taken as it is.
        assert_eq!(cut(&["Komm", " her", " und", " hör", " gut", " zu", " mir", "."]), ("Komm her und hör gut zu mir.".into(), false));
        // A stream that stops by itself after the mark without a clean end is cut the same way:
        // the running sentence is dropped.
        assert_eq!(run(100, &["Er", " geht", "."]).0, "Er geht.", "short: all of it");
        assert_eq!(
            cut(&["Er", " geht", " fort", ".", " Sie", " bleibt", " noch", " da"]),
            ("Er geht fort. Sie".into(), false),
            "no end since the mark: what went on before it stays"
        );
    }

    #[test]
    fn without_any_end_since_the_mark_the_held_text_is_dropped() {
        // Target 8, mark 6: nothing ends after it. What was handed on before the mark stays.
        let (out, ended) = run(8, &["Sie", " geht", " zur", " Tür", " und", " öffnet", " sie", " ganz", " langsam"]);
        assert_eq!((out.as_str(), ended), ("Sie geht zur Tür und", false), "clean: the held part is dropped, nothing added");
        // Open there: the marks are closed.
        let (out, _) = run(8, &["„Ich", " gehe", " zur", " Tür", " und", " öffne", " sie", " ganz", " langsam"]);
        assert_eq!(out, "„Ich gehe zur Tür und“");
        let (out, _) = run(8, &["*Sie", " geht", " zur", " Tür", " und", " öffnet", " sie", " ganz", " langsam"]);
        assert_eq!(out, "*Sie geht zur Tür und*");
    }

    #[test]
    fn a_reply_written_on_counts_what_its_base_left_open() {
        let run_on = |base: &str, pieces: &[&str]| {
            let mut s = SoftStop::new(3).with_base(base);
            let mut out = String::new();
            for (n, p) in pieces.iter().enumerate() {
                match s.push(p, n as u64 + 1) {
                    Step::Go(t) => out.push_str(&t),
                    Step::End(t) => return out + &t,
                }
            }
            out + &s.finish()
        };
        let pieces = ["es", ".", " Geh", " weg", ".“", " Dann"];
        assert_eq!(run_on("Er sagt: „Vergiss ", &pieces), "es. Geh weg.“", "the speech of the base closes first");
        assert_eq!(run_on("Er sagt: „Vergiss es.“ Sie ", &pieces), "es.", "a clean base: the first end");
    }

    #[test]
    fn no_device_sees_text_the_reply_then_loses() {
        // Target 4: the mark is token 3.
        let mut s = SoftStop::new(4);
        assert_eq!(s.push("\"Yes", 1), Step::Go("\"Yes".into()));
        assert_eq!(s.push(" no", 2), Step::Go(" no".into()));
        assert_eq!(s.push(".\"", 3), Step::Go(String::new()), "mark reached: the running sentence and its end marks wait");
        assert_eq!(s.push(" She", 4), Step::End(".\"".into()));
        // A finished sentence inside speech goes on, the new one waits.
        let mut s = SoftStop::new(4);
        assert_eq!(s.push("„Yes", 1), Step::Go("„Yes".into()));
        assert_eq!(s.push(" no", 2), Step::Go(" no".into()));
        assert_eq!(s.push(".", 3), Step::Go(String::new()), "mark reached: the end mark waits");
        assert_eq!(s.push(" She", 4), Step::Go(".".into()), "the end is confirmed but not clean: it goes on, the new sentence waits");
        assert_eq!(s.push(" runs", 5), Step::Go(String::new()));
        assert_eq!(s.finish(), "“", "the running sentence is dropped, the speech closed");
    }

    /// Splits a text into pieces like tokens: a word with its space, every mark of its own.
    fn tokens(text: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        let mut word = String::new();
        for c in text.chars() {
            if matches!(c, '.' | '!' | '?' | '…' | '“' | '”' | '„' | '«' | '»' | '"' | '*' | '_' | ',' | ':' | '\n') {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                out.push(c.to_string());
            } else if c == ' ' {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                word.push(' ');
            } else {
                word.push(c);
            }
        }
        if !word.is_empty() {
            out.push(word);
        }
        out
    }

    #[test]
    fn a_reply_is_never_longer_than_its_length_and_never_ends_open() {
        let texts = [
            "*Sie lehnt sich zurück.* „Ich habe dir gesagt, dass es nicht leicht wird. Aber du wolltest ja nicht hören.“ Sie lacht leise. »Na gut«, sagt sie, «dann eben anders». *Dann steht sie auf und geht zum Fenster. Draußen regnet es.* \"Hörst du das?\" Sie schweigt.",
            "He stands. “Listen to me, please. I mean it. Nothing else matters now!” *He turns away and walks to the door, slowly. It creaks.* “Fine,” he says. “Have it your way.”",
            "Она встала. «Уходи отсюда. Я не хочу тебя видеть. Никогда больше.» *Она отвернулась к окну и замолчала.* „Хорошо“, сказала она. «Как хочешь».",
            "*Se acerca despacio. Mira el suelo. Respira hondo.* «Tengo que decirte algo importante. No sé cómo empezar.» Calla un momento. «Te quiero. Eso es todo.»",
            "Ein Satz ohne Ende und ohne Punkt der immer weiter geht und weiter geht und weiter geht und nie aufhört bis zum Schluss",
        ];
        for text in texts {
            let pieces = tokens(text);
            for target in 1..=pieces.len() as i64 {
                // The model server stops after `target` tokens.
                let fed: Vec<&str> = pieces.iter().take(target as usize).map(String::as_str).collect();
                let joined = fed.concat();
                let (out, _) = run(target, &fed);
                // Everything is a prefix of what was written, apart from closing marks at the end.
                let common = out.chars().zip(joined.chars()).take_while(|(a, b)| a == b).count();
                let added: String = out.chars().skip(common).collect();
                assert!(
                    added.chars().count() <= 4 && added.chars().all(|c| matches!(c, '“' | '”' | '«' | '»' | '"' | '*' | '_' | '`')),
                    "target {target}: {out:?} is no cut of {joined:?}"
                );
                // Never longer than the length: the tokens of the written part do not exceed it.
                let kept: String = out.chars().take(common).collect();
                let kept_tokens = tokens(&kept).len() as i64;
                assert!(kept_tokens <= target, "target {target}: {kept_tokens} tokens in {out:?}");
                // Nothing stays open (once the mark is reached; below it the stream is as written).
                if target >= 1 && fed.len() as u64 >= SoftStop::new(target).mark {
                    assert!(open_at(&out, &[], out.len()).is_empty(), "target {target}: {out:?} ends open");
                }
            }
        }
    }

    #[test]
    fn what_a_device_got_is_always_part_of_the_final_reply() {
        // Whatever is handed on is never taken back: the reply is the concatenation of the steps.
        let text = "*Sie lehnt sich zurück.* „Ich habe dir gesagt, dass es nicht leicht wird. Aber du wolltest ja nicht hören.“ Sie lacht leise. »Na gut«, sagt sie.";
        let pieces = tokens(text);
        for target in 1..=pieces.len() as i64 {
            let mut s = SoftStop::new(target);
            let mut shown = String::new();
            let mut from_mark: Option<usize> = None;
            for (n, p) in pieces.iter().enumerate() {
                let step = s.push(p, n as u64 + 1);
                let (Step::Go(t) | Step::End(t)) = &step;
                shown.push_str(t);
                if from_mark.is_none() && n as u64 + 1 >= s.mark {
                    from_mark = Some(shown.len());
                }
                // After the mark the running sentence is never shown: what a device has ends at
                // a sentence end (or at the point where the mark was reached).
                if let (Some(m), Step::Go(_)) = (from_mark, &step) {
                    if shown.len() > m {
                        let (ends, _) = scan(&shown, &[], 0, true);
                        assert!(ends.last().is_some_and(|e| e.at == shown.len()), "target {target}: {shown:?} ends mid-sentence");
                    }
                }
                if matches!(step, Step::End(_)) {
                    break;
                }
            }
        }
    }
}
