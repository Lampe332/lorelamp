//! The hub: storage, single-flight generation against llama-server, broadcast of events to all
//! connected devices.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use futures_util::{FutureExt, StreamExt};
use rp_core::card::Card;
use rp_core::length::{SoftStop, Step};
use rp_core::prompt::{self, ChatMsg, Cut, Language, Role, Turn, UserFigure};
use rp_core::sampler::{Sampler, DEFAULT_N_PREDICT};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Notify};

use crate::engine::idle::{self, Idle};
use crate::logs::{Entry, Level, Logger};
use crate::store::{self, Res, Store, Timings};

/// The one running generation (the model server works on one request at a time).
struct Gen {
    chat_id: i64,
    msg_id: i64,
    character_id: i64,
    started: Instant,
    content: String,
    /// `notify_one` aborts it.
    cancel: Arc<Notify>,
    kind: Kind,
}

/// What a generation writes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    /// The reply to a new message of the user (a new, empty message).
    Reply,
    /// Another variant of the chat's last reply ("Neu generieren").
    Variant,
    /// More text at the end of the chat's last reply ("Weiter schreiben"); `base` is the length
    /// of the text it starts from.
    Continue { base: usize },
}

impl Kind {
    fn mode(self) -> &'static str {
        match self {
            Kind::Reply => "reply",
            Kind::Variant => "variant",
            Kind::Continue { .. } => "continue",
        }
    }
}

struct Inner {
    store: Store,
    gen: Option<Gen>,
}

pub struct Hub {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<String>,
    /// llama-server's base URL; follows the port the supervisor starts the model server on.
    llama: Mutex<String>,
    http: reqwest::Client,
    log: Logger,
    /// Unloaded to free the graphics memory, on its way up, quiet since when. Locked after
    /// `inner` when both are needed.
    idle: Mutex<Idle>,
    /// One "minute" of the setting `idle_unload_min` (60 s; a test server shortens it).
    idle_minute: Duration,
    /// Wakes the model supervisor (set when this server manages the model server).
    waker: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// A benchmark runs: what the supervisor runs instead of the active profile. Begun and checked
    /// by `send` while `inner` is held (locked after it), so a reply and a benchmark never overlap.
    bench: Mutex<Option<BenchRun>>,
    /// The model works outside any chat (the card builder writes a field): no reply, benchmark or
    /// unload starts meanwhile, and only one such job runs at a time. Locked after `inner`.
    side: Mutex<bool>,
    /// How many tokens the model's tokenizer makes of a message's text (see `fit`).
    tokens: Mutex<TokenCache>,
}

/// Token counts of message texts, for the model they were counted with: a long chat is not
/// counted again on every reply. Never locked together with another lock of the hub.
#[derive(Default)]
struct TokenCache {
    /// The model file the counts belong to (another model, another tokenizer: counted anew).
    model: String,
    counts: std::collections::HashMap<u64, usize>,
}

/// At most this many texts are kept counted (beyond that the cache starts again).
const TOKEN_CACHE_MAX: usize = 50_000;

/// Tokens kept free besides the reply (rounding of the estimate, the BOS token).
const CONTEXT_MARGIN: usize = 16;

/// How many tokens the prompt may take in a context of `n_ctx` tokens: what is left after room
/// for the reply (the hard cap, `length::hard_cap`: the profile's answer length `n_predict`, or
/// with `reasoning` the wider cap; at most half the context) and a small margin.
pub(crate) fn prompt_budget(n_ctx: u64, n_predict: Option<i64>, reasoning: bool) -> usize {
    let n_ctx = usize::try_from(n_ctx).unwrap_or(usize::MAX);
    let cap = rp_core::length::hard_cap(n_predict.unwrap_or(DEFAULT_N_PREDICT), reasoning);
    let reply = usize::try_from(cap).unwrap_or(usize::MAX).min(n_ctx / 2);
    n_ctx.saturating_sub(reply + CONTEXT_MARGIN)
}

/// Tokens a text takes when the model server cannot count them: one per three bytes of UTF-8,
/// more than any common tokenizer makes of German, English, Spanish or Russian text.
fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(3)
}

/// Whether llama-server refused a prompt as longer than its context.
fn exceeds_context(body: &str) -> bool {
    body.contains("exceed_context_size_error") || body.contains("exceeds the available context size")
}

/// The hold of a job outside any chat on the model server ([`Hub::side_begin`]); dropping it
/// frees the model server again.
pub struct SideJob(Arc<Hub>);

impl Drop for SideJob {
    fn drop(&mut self) {
        *lock(&self.0.side) = false;
    }
}

/// The benchmark's hold on the model server.
#[derive(Debug, Clone)]
pub struct BenchRun {
    pub profile_id: i64,
    /// The candidate to run; None: none (between two candidates).
    pub spec: Option<crate::engine::LaunchSpec>,
}

/// How long a message sent while the model loads waits for it.
const LOAD_WAIT: Duration = Duration::from_secs(600);

/// The longest message (characters after trimming) that can be sent or stored by an edit: room
/// for long roleplay posts, but no megabytes in the database, the search index, every device and
/// the prompt. Longer: error `too_long`.
pub const MESSAGE_MAX_CHARS: usize = 64_000;

/// The error of a generation whose prompt does not fit into the model's context even with the
/// history cut down to the newest message (the head, a persona or that message alone is too
/// long): nothing is written. REST answers it with 413.
pub const CONTEXT_FULL: &str = "context_full";

/// Locks `m` even after a panic elsewhere left it poisoned. The hub's state stays usable then:
/// every change under these locks is a single store call or a plain assignment, and a reply that
/// was being written is ended by `finish` like any failed one.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn notice(code: &str) -> String {
    json!({ "type": "notice", "code": code }).to_string()
}

/// Everything a chat's prompt is built from, read when the message was sent.
struct Source {
    lang: Language,
    card: Card,
    /// The chat's persona: its name is `{{user}}`.
    user_name: String,
    figure: UserFigure,
    /// The whole history the reply follows (it ends with the user's message).
    turns: Vec<Turn>,
    /// The answer length of the active profile in tokens (`Sampler::reply_length`).
    length: i64,
}

impl Source {
    /// The prompt with the part of the history that `cut` keeps.
    fn build(&self, cut: Cut) -> Vec<ChatMsg> {
        let turns = if cut == Cut::NONE { self.turns.clone() } else { cut.apply(&self.turns) };
        prompt::build_reply(self.lang, &self.card, &self.user_name, &self.figure, &turns, self.length)
    }

    /// The prompt without any history (`prompt::frame`).
    fn frame(&self) -> Vec<ChatMsg> {
        prompt::frame(self.lang, &self.card, &self.user_name, &self.figure, self.length)
    }
}

/// The prompt of one generation as it goes to the model: the messages and what the chat template
/// made of them.
struct Fitted {
    rendered: String,
}

struct Prepared {
    chat_id: i64,
    msg_id: i64,
    kind: Kind,
    /// The text the message starts with (`Kind::Continue`: what it holds already). It follows the
    /// prompt as the open turn of the character, so the model goes on from its last word.
    start: String,
    /// What the log says about the start (ids and sizes only).
    log: Value,
    character_id: i64,
    source: Source,
    stop: Vec<String>,
    /// Of the active model profile at the moment the message was sent.
    sampler: Sampler,
    reasoning: bool,
    /// The context size the active profile asks for (`-c`), used when the model server does not
    /// say its own.
    ctx_hint: Option<u64>,
}

impl Inner {
    /// Stores the user's message plus an empty reply and builds the prompt. None: no such chat.
    fn prepare(&mut self, chat_id: i64, text: &str, tx: &broadcast::Sender<String>) -> Res<Option<Prepared>> {
        let Some((character, _)) = self.store.chat_character_card(chat_id)? else { return Ok(None) };
        let persona = self.store.chat_persona(chat_id)?;
        let (sampler, reasoning) = self.store.sampling()?;
        let ctx_hint = self.ctx_hint()?;
        let user_msg = self.store.add_user_message(chat_id, text, persona.id)?;
        let source = self.source(chat_id, None)?.ok_or("chat vanished")?;
        let reply = self.store.add_message(chat_id, "assistant", "")?;
        let _ = tx.send(json!({ "type": "message", "chat_id": chat_id, "message": user_msg }).to_string());
        let _ = tx.send(json!({ "type": "assistant_start", "chat_id": chat_id, "id": reply.id }).to_string());
        Ok(Some(Prepared {
            chat_id,
            msg_id: reply.id,
            kind: Kind::Reply,
            start: String::new(),
            log: json!({
                "user_message_id": user_msg.id, "user_chars": text.chars().count(),
                "reply_message_id": reply.id, "history_turns": source.turns.len(),
            }),
            character_id: character.info.id,
            stop: prompt::stop_strings(&source.user_name),
            source,
            sampler,
            reasoning,
            ctx_hint,
        }))
    }

    /// The context size the active profile starts the model server with (`-c`), if it says one.
    fn ctx_hint(&self) -> Res<Option<u64>> {
        Ok(self.store.launch_spec()?.and_then(|spec| rp_core::llama_args::ctx_from_args(&spec.args)))
    }

    /// The chat's last reply written again (`again`: another variant) or written on. It must be
    /// the last message, the character's, and follow a message of the user: the prompt is then
    /// the very one that reply was written from (layout B: same head, same history, same tail).
    /// Err = error code.
    fn prepare_last(&self, msg_id: i64, again: bool) -> Result<(i64, Prepared), &'static str> {
        let chat_id = self.store.message_chat(msg_id).map_err(storage_err)?.ok_or("message_missing")?;
        let messages = self.store.messages(chat_id).map_err(storage_err)?;
        let Some((last, before)) = messages.split_last().filter(|(last, _)| last.id == msg_id) else { return Err("not_last") };
        if last.role != "assistant" || before.last().is_none_or(|m| m.role != "user") {
            return Err("not_reply");
        }
        let (character, _) = self.store.chat_character_card(chat_id).map_err(storage_err)?.ok_or("message_missing")?;
        let (sampler, reasoning) = self.store.sampling().map_err(storage_err)?;
        let ctx_hint = self.ctx_hint().map_err(storage_err)?;
        let source = self.source(chat_id, Some(msg_id)).map_err(storage_err)?.ok_or("message_missing")?;
        let (kind, start) = if again { (Kind::Variant, String::new()) } else { (Kind::Continue { base: last.content.len() }, last.content.clone()) };
        let prepared = Prepared {
            chat_id,
            msg_id,
            kind,
            log: json!({ "mode": kind.mode(), "reply_message_id": msg_id, "start_chars": start.chars().count(), "history_turns": source.turns.len() }),
            start,
            character_id: character.info.id,
            stop: prompt::stop_strings(&source.user_name),
            source,
            sampler,
            reasoning,
            ctx_hint,
        };
        Ok((chat_id, prepared))
    }

    /// The whole prompt for the next reply in this chat (no cut, see `Hub::fit`), plus the name
    /// of the persona. None: no such chat.
    #[cfg(test)]
    fn prompt(&self, chat_id: i64, before: Option<i64>) -> Res<Option<(Vec<ChatMsg>, String)>> {
        Ok(self.source(chat_id, before)?.map(|s| (s.build(Cut::NONE), s.user_name)))
    }

    /// What the prompt for the next reply in this chat is built from (`before`: for the reply that
    /// follows the messages before this one), as the chat's persona sees it (its name is
    /// `{{user}}`, its fields form the block in the head). None: no such chat.
    fn source(&self, chat_id: i64, before: Option<i64>) -> Res<Option<Source>> {
        let Some((character, _)) = self.store.chat_character_card(chat_id)? else { return Ok(None) };
        let persona = self.store.chat_persona(chat_id)?;
        let turns: Vec<Turn> = self
            .store
            .messages(chat_id)?
            .into_iter()
            .filter(|m| before.is_none_or(|id| m.id < id))
            .map(|m| Turn { role: if m.role == "user" { Role::User } else { Role::Assistant }, content: m.content })
            .collect();
        // Card vs. persona about {{user}}: the character's override, else the persona's setting.
        let mut figure = persona.figure();
        figure.priority = character.info.user_priority.unwrap_or(persona.card_priority);
        Ok(Some(Source {
            lang: self.store.language()?,
            card: character.card,
            user_name: persona.name,
            figure,
            turns,
            length: self.store.sampling()?.0.reply_length(),
        }))
    }

    /// The model is writing in this chat right now.
    fn writing_in(&self, chat_id: i64) -> bool {
        self.gen.as_ref().is_some_and(|gen| gen.chat_id == chat_id)
    }

    /// The `history` event: everything a device needs to show a chat. None: no such chat.
    fn snapshot(&self, chat_id: i64) -> Res<Option<String>> {
        let Some((character, _)) = self.store.chat_character_card(chat_id)? else { return Ok(None) };
        let Some(chat) = self.store.chat_info(chat_id)? else { return Ok(None) };
        let mut messages = self.store.messages(chat_id)?;
        let gen = self.gen.as_ref().filter(|gen| gen.chat_id == chat_id);
        if let Some(gen) = gen {
            if let Some(m) = messages.iter_mut().find(|m| m.id == gen.msg_id) {
                m.content = gen.content.clone();
            }
        }
        let snapshot = json!({
            "type": "history",
            "chat_id": chat_id,
            "chat": chat,
            "character": character.info,
            "messages": messages,
            "busy": self.gen.is_some(),
            "generating": gen.map(|gen| gen.msg_id),
        });
        Ok(Some(snapshot.to_string()))
    }
}

/// A message's text as it is stored: trimmed. Err: `empty`, `too_long` (see `MESSAGE_MAX_CHARS`).
fn message_text(text: &str) -> Result<&str, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err("empty");
    }
    // Bytes first: a text with at most as many bytes as the limit cannot have more characters.
    if text.len() > MESSAGE_MAX_CHARS && text.chars().count() > MESSAGE_MAX_CHARS {
        return Err("too_long");
    }
    Ok(text)
}

fn storage_err(e: impl std::fmt::Display) -> &'static str {
    eprintln!("storage error: {e}");
    "storage"
}

impl Hub {
    pub fn new(store: Store, llama: String, log: Logger) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Hub {
            inner: Mutex::new(Inner { store, gen: None }),
            tx,
            llama: Mutex::new(llama.trim_end_matches('/').to_string()),
            http: reqwest::Client::new(),
            log,
            idle: Mutex::new(Idle::default()),
            idle_minute: Duration::from_secs(60),
            waker: OnceLock::new(),
            bench: Mutex::new(None),
            side: Mutex::new(false),
            tokens: Mutex::default(),
        }
    }

    /// One minute of the setting `idle_unload_min` lasts `minute` (tests: seconds).
    pub fn with_idle_minute(mut self, minute: Duration) -> Self {
        self.idle_minute = minute;
        self
    }

    /// How the hub wakes the model supervisor when the model is wanted again.
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        let _ = self.waker.set(Box::new(wake));
    }

    fn wake_engine(&self) {
        if let Some(wake) = self.waker.get() {
            wake();
        }
    }

    /// The model is unloaded: no model server runs until the next message or "load".
    pub fn unloaded(&self) -> bool {
        lock(&self.idle).unloaded
    }

    /// Unload the model now. Err("busy"): a reply is being written.
    pub fn unload(&self) -> Result<(), &'static str> {
        let g = lock(&self.inner);
        if g.gen.is_some() || *lock(&self.side) {
            return Err("busy");
        }
        lock(&self.idle).unload();
        drop(g);
        self.wake_engine();
        Ok(())
    }

    /// Load the model again. Answers whether it was unloaded.
    pub fn load(&self) -> bool {
        let woke = lock(&self.idle).load();
        if woke {
            self.woke();
        }
        woke
    }

    /// Every device shows "Modell lädt …" at once, and the supervisor starts the model now.
    fn woke(&self) {
        let _ = self.tx.send(json!({ "type": "engine", "event": "waking", "profile_id": null }).to_string());
        self.wake_engine();
    }

    /// Asked by the supervisor on every check: unloaded? After `minutes` quiet minutes (0 = never)
    /// without a reply being written, this unloads first.
    pub fn idle_check(&self, minutes: u32) -> bool {
        let g = lock(&self.inner);
        let busy = g.gen.is_some() || *lock(&self.side);
        let mut idle = lock(&self.idle);
        if idle.due(idle::limit(minutes, self.idle_minute), busy, Instant::now()) {
            idle.unload();
            self.log.write(
                Entry::new(Level::Info, "engine", "unload", "model unloaded after the quiet time").data(json!({ "idle_unload_min": minutes })),
            );
        }
        idle.unloaded
    }

    /// A message sent while the model is on its way up waits until it answers (at most
    /// `LOAD_WAIT`); if it does not come up, the request below fails as it would have anyway.
    async fn wait_ready(&self) {
        let deadline = Instant::now() + LOAD_WAIT;
        while lock(&self.idle).waking && Instant::now() < deadline {
            if self.engine_status().await == "ready" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    pub fn log(&self) -> &Logger {
        &self.log
    }

    /// Logs the outcome of a change to a chat's messages (ids and counts only, never text).
    fn log_change<R>(&self, what: &str, chat_id: Option<i64>, data: Value, result: &Result<R, &'static str>) {
        let e = match result {
            Ok(_) => Entry::new(Level::Info, "chat", "message", what.to_string()),
            Err(code) => Entry::new(Level::Warn, "chat", "message", format!("{what} refused: {code}")),
        };
        let e = e.data(data);
        self.log.write(match chat_id {
            Some(id) => e.chat(id),
            None => e,
        });
    }

    /// Status of the model server: "ready", "loading" (llama-server answers 503 while it loads
    /// the model), "error" (other HTTP status) or "offline" (not reachable).
    pub async fn engine_status(&self) -> &'static str {
        let req = self
            .http
            .get(format!("{}/health", self.llama_url()))
            .timeout(std::time::Duration::from_secs(2));
        match req.send().await {
            Ok(r) if r.status().is_success() => "ready",
            Ok(r) if r.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE => "loading",
            Ok(_) => "error",
            Err(_) => "offline",
        }
    }

    /// llama-server's `/props` (model file, context, build). None while it is not there.
    pub async fn props(&self) -> Option<Value> {
        let req = self.http.get(format!("{}/props", self.llama_url())).timeout(std::time::Duration::from_secs(2));
        req.send().await.ok()?.error_for_status().ok()?.json().await.ok()
    }

    /// Where the model server answers (llama-server's base URL).
    pub fn llama_url(&self) -> String {
        lock(&self.llama).clone()
    }

    /// The managed model server now listens on this port (set by the supervisor when it starts
    /// one there or finds a llama-server started elsewhere).
    pub fn set_llama_port(&self, port: u16) {
        *lock(&self.llama) = format!("http://127.0.0.1:{port}");
    }

    /// A benchmark takes over the model server. Err(`busy`): a reply is being written;
    /// Err(`benchmark`): another one runs.
    pub fn bench_begin(&self, profile_id: i64) -> Result<(), &'static str> {
        let g = lock(&self.inner);
        if g.gen.is_some() || *lock(&self.side) {
            return Err("busy");
        }
        let mut b = lock(&self.bench);
        if b.is_some() {
            return Err("benchmark");
        }
        *b = Some(BenchRun { profile_id, spec: None });
        drop(b);
        drop(g);
        self.wake_engine();
        Ok(())
    }

    /// The candidate the supervisor should run now (None: stop the model server).
    pub fn bench_set(&self, spec: Option<crate::engine::LaunchSpec>) {
        if let Some(b) = lock(&self.bench).as_mut() {
            b.spec = spec;
        }
        self.wake_engine();
    }

    /// The benchmark is over: the supervisor runs the active profile again (as before it).
    pub fn bench_end(&self) {
        *lock(&self.bench) = None;
        self.wake_engine();
    }

    /// The benchmark that holds the model server, if one does.
    pub fn bench_run(&self) -> Option<BenchRun> {
        lock(&self.bench).clone()
    }

    /// The model is writing right now: a reply, or a job outside any chat.
    pub fn generating(&self) -> bool {
        let g = lock(&self.inner);
        g.gen.is_some() || *lock(&self.side)
    }

    /// A job outside any chat takes the model server (one generation at a time, as a reply does).
    /// Err(`busy`): a reply or another such job is being written; Err(`benchmark`): a benchmark
    /// holds the model server.
    pub fn side_begin(self: &Arc<Self>) -> Result<SideJob, &'static str> {
        let g = lock(&self.inner);
        self.free(&g)?;
        let mut side = lock(&self.side);
        if *side {
            return Err("busy");
        }
        *side = true;
        Ok(SideJob(self.clone()))
    }

    /// The model server changed (`starting`, `switching`, `ready`, `crashed`, `failed`, `foreign`,
    /// `port_busy`): every device gets `{"type":"engine","event":…,"profile_id":…,"port":…}` and
    /// asks `/api/engine` again.
    /// `port`: where the model server listens (`port_busy`: the port another program holds).
    pub fn engine_event(&self, event: &str, profile_id: Option<i64>, port: Option<u16>) {
        lock(&self.idle).event(event, Instant::now());
        let _ = self.tx.send(json!({ "type": "engine", "event": event, "profile_id": profile_id, "port": port }).to_string());
    }

    /// Model profiles or engines changed (also which one is active): `{"type":"models"}`.
    pub fn models_changed(&self) {
        let _ = self.tx.send(json!({ "type": "models" }).to_string());
    }

    /// A ready event for every device (e.g. `{"type":"engine_download",…}`).
    pub fn send_event(&self, event: &serde_json::Value) {
        let _ = self.tx.send(event.to_string());
    }

    /// Progress of an import: `{"type":"import","job","done","total","finished"}` (`total` grows
    /// while files arrive and zips are unpacked; `finished` once everything is stored).
    pub fn import_progress(&self, job: &str, done: usize, total: usize, finished: bool) {
        let _ = self.tx.send(json!({ "type": "import", "job": job, "done": done, "total": total, "finished": finished }).to_string());
    }

    /// Any other event for every device (model downloads, catalog refreshed).
    pub fn notify(&self, event: &Value) {
        let _ = self.tx.send(event.to_string());
    }

    /// Run a storage operation (never held across an await).
    pub fn store<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R {
        f(&mut lock(&self.inner).store)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// Tell every device that characters, chats or settings changed.
    pub fn library_changed(&self) {
        let _ = self.tx.send(json!({ "type": "library" }).to_string());
    }

    /// The latest chat with a character, or a new one (`fresh`, or none exists yet) that opens
    /// with the card's greeting. None: no such character.
    pub fn open_chat(&self, character_id: i64, fresh: bool) -> Res<Option<i64>> {
        let mut g = lock(&self.inner);
        let Some(character) = g.store.character(character_id)? else { return Ok(None) };
        if !fresh {
            if let Some(id) = g.store.latest_chat(character_id)? {
                return Ok(Some(id));
            }
        }
        // The greeting greets the persona the new chat will play with; the card's alternate
        // greetings are its variants, to swipe through like a reply's.
        let persona = g.store.persona_for_new_chat(character_id)?;
        let greetings = prompt::greetings(&character.card, &persona.name);
        Ok(Some(g.store.create_chat_with_greetings(character_id, &greetings)?))
    }

    /// Binds a chat to a persona (`Some`) or lets it follow the active one (`None`); every device
    /// gets the chat's new `history` snapshot (the caller sends the `library` event). Allowed
    /// while the model writes: the running reply keeps the prompt it started with.
    pub fn set_chat_persona(&self, chat_id: i64, persona_id: Option<i64>) -> Res<store::Bind> {
        let mut g = lock(&self.inner);
        let bound = g.store.set_chat_persona(chat_id, persona_id)?;
        if bound == store::Bind::Done {
            if let Some(snapshot) = g.snapshot(chat_id)? {
                let _ = self.tx.send(snapshot);
            }
        }
        Ok(bound)
    }

    /// Snapshot of a chat plus a fresh subscription, both under the same lock: events are
    /// broadcast while holding it, so none is lost or applied twice. None: no such chat.
    pub fn open_view(&self, chat_id: i64) -> Res<Option<(String, broadcast::Receiver<String>)>> {
        let g = lock(&self.inner);
        Ok(g.snapshot(chat_id)?.map(|snapshot| (snapshot, self.tx.subscribe())))
    }

    /// Changes the messages of the chat that holds `msg_id`, then sends every device the chat's
    /// new state. Err = error code: no such message, or the model is writing in that chat.
    fn change<R>(&self, msg_id: i64, f: impl FnOnce(&mut Store, i64) -> Res<R>) -> Result<R, &'static str> {
        let mut g = lock(&self.inner);
        let chat_id = g.store.message_chat(msg_id).map_err(storage_err)?.ok_or("message_missing")?;
        self.change_chat(&mut g, chat_id, |store| f(store, chat_id))
    }

    fn change_chat<R>(&self, g: &mut Inner, chat_id: i64, f: impl FnOnce(&mut Store) -> Res<R>) -> Result<R, &'static str> {
        if g.writing_in(chat_id) {
            return Err("busy");
        }
        let out = f(&mut g.store).map_err(storage_err)?;
        if let Some(snapshot) = g.snapshot(chat_id).map_err(storage_err)? {
            let _ = self.tx.send(snapshot);
        }
        let _ = self.tx.send(json!({ "type": "library" }).to_string());
        Ok(out)
    }

    /// Replaces the text of one message (the user's or the character's).
    pub fn edit_message(&self, msg_id: i64, text: &str) -> Result<(), &'static str> {
        let text = message_text(text)?;
        let r = self.change(msg_id, |store, chat_id| store.edit_message(msg_id, text).map(|_| chat_id));
        self.log_change("message edited", r.as_ref().ok().copied(), json!({ "message_id": msg_id, "chars": text.chars().count() }), &r);
        r.map(|_| ())
    }

    /// Makes another of a message's variants the active one (its text becomes the message's).
    pub fn set_swipe(&self, msg_id: i64, index: i64) -> Result<(), &'static str> {
        let r = self.set_swipe_inner(msg_id, index);
        self.log_change("variant chosen", r.as_ref().ok().copied(), json!({ "message_id": msg_id, "swipe_id": index }), &r);
        r.map(|_| ())
    }

    fn set_swipe_inner(&self, msg_id: i64, index: i64) -> Result<i64, &'static str> {
        let mut g = lock(&self.inner);
        let chat_id = g.store.message_chat(msg_id).map_err(storage_err)?.ok_or("message_missing")?;
        if g.writing_in(chat_id) {
            return Err("busy");
        }
        let (swipes, _) = g.store.message_swipes(msg_id).map_err(storage_err)?.ok_or("no_swipes")?;
        let i = usize::try_from(index).ok().filter(|&i| i < swipes.len()).ok_or("swipe_out_of_range")?;
        self.change_chat(&mut g, chat_id, |store| store.set_swipe(msg_id, i, &swipes[i]))?;
        Ok(chat_id)
    }

    /// Removes a message and everything after it. Answers the chat and how many were removed.
    pub fn rewind(&self, msg_id: i64) -> Result<(i64, usize), &'static str> {
        let r = self.change(msg_id, |store, chat_id| Ok((chat_id, store.rewind(chat_id, msg_id)?)));
        let removed = r.as_ref().map_or(0, |(_, n)| *n);
        self.log_change("rewound", r.as_ref().ok().map(|(c, _)| *c), json!({ "from_message_id": msg_id, "removed": removed }), &r);
        r
    }

    /// Brings back what the last rewind in this chat removed.
    pub fn undo_rewind(&self, chat_id: i64) -> Result<(), &'static str> {
        let r = self.undo_rewind_inner(chat_id);
        self.log_change("rewind undone", Some(chat_id), json!({ "restored": r.as_ref().copied().unwrap_or(0) }), &r);
        r.map(|_| ())
    }

    fn undo_rewind_inner(&self, chat_id: i64) -> Result<usize, &'static str> {
        let mut g = lock(&self.inner);
        if g.store.chat_character(chat_id).map_err(storage_err)?.is_none() {
            return Err("chat_missing");
        }
        match self.change_chat(&mut g, chat_id, |store| store.undo_rewind(chat_id))? {
            0 => Err("undo_gone"),
            n => Ok(n),
        }
    }

    /// A new chat that holds this one up to and including `msg_id`. Answers the new chat.
    pub fn fork(&self, msg_id: i64, title: &str) -> Result<i64, &'static str> {
        let r = self.fork_inner(msg_id, title);
        let chat = r.as_ref().ok().map(|(chat, _)| *chat);
        let new_chat = r.as_ref().ok().map(|(_, fork)| *fork);
        self.log_change("chat forked", chat, json!({ "upto_message_id": msg_id, "new_chat_id": new_chat }), &r);
        r.map(|(_, fork)| fork)
    }

    fn fork_inner(&self, msg_id: i64, title: &str) -> Result<(i64, i64), &'static str> {
        let mut g = lock(&self.inner);
        let chat_id = g.store.message_chat(msg_id).map_err(storage_err)?.ok_or("message_missing")?;
        if g.writing_in(chat_id) {
            return Err("busy");
        }
        let fork = g.store.fork_chat(chat_id, msg_id, title).map_err(storage_err)?;
        let _ = self.tx.send(json!({ "type": "library" }).to_string());
        Ok((chat_id, fork))
    }

    /// Start a generation for a new user message. Err = notice code for that client only.
    pub fn send(self: &Arc<Self>, chat_id: i64, text: String) -> Result<(), &'static str> {
        let r = self.send_inner(chat_id, text);
        if let Err(code) = r {
            self.log.write(Entry::new(Level::Warn, "chat", "generate", format!("send refused: {code}")).chat(chat_id));
        }
        r
    }

    fn send_inner(self: &Arc<Self>, chat_id: i64, text: String) -> Result<(), &'static str> {
        let text = message_text(&text)?;
        let mut g = lock(&self.inner);
        self.free(&g)?;
        let prepared = match g.prepare(chat_id, text, &self.tx) {
            Ok(Some(p)) => p,
            Ok(None) => return Err("chat_missing"),
            Err(e) => {
                eprintln!("storage error: {e}");
                return Err("storage");
            }
        };
        self.start(g, chat_id, prepared);
        Ok(())
    }

    /// Writes another variant of the chat's last reply: the same prompt as for the reply before,
    /// the new text becomes the active variant (`messages.swipes`), the earlier ones stay. Every
    /// device gets the chat's `history` snapshot at the start and at the end. Err = error code:
    /// `busy` while the model writes, `not_last`, `not_reply`, `message_missing`.
    pub fn regenerate(self: &Arc<Self>, msg_id: i64) -> Result<(), &'static str> {
        self.write_last(msg_id, true)
    }

    /// Lets the model go on writing at the end of the chat's last reply. Same rules as `regenerate`.
    pub fn continue_reply(self: &Arc<Self>, msg_id: i64) -> Result<(), &'static str> {
        self.write_last(msg_id, false)
    }

    fn write_last(self: &Arc<Self>, msg_id: i64, again: bool) -> Result<(), &'static str> {
        let what = if again { "regenerate" } else { "continue" };
        let g = lock(&self.inner);
        let r = self.free(&g).and_then(|()| g.prepare_last(msg_id, again));
        match r {
            Ok((chat_id, prepared)) => {
                self.start(g, chat_id, prepared);
                Ok(())
            }
            Err(code) => {
                drop(g);
                self.log.write(Entry::new(Level::Warn, "chat", "generate", format!("{what} refused: {code}")).data(json!({ "message_id": msg_id })));
                Err(code)
            }
        }
    }

    /// The model server is free for a generation. Err(`busy`): the model writes (in any chat);
    /// Err(`benchmark`): it runs test settings and restarts often.
    fn free(&self, g: &Inner) -> Result<(), &'static str> {
        if g.gen.is_some() || *lock(&self.side) {
            return Err("busy");
        }
        if lock(&self.bench).is_some() {
            return Err("benchmark");
        }
        Ok(())
    }

    /// Starts the prepared generation (the caller checked `free` under the same lock).
    fn start(self: &Arc<Self>, mut g: std::sync::MutexGuard<'_, Inner>, chat_id: i64, prepared: Prepared) {
        let cancel = Arc::new(Notify::new());
        g.gen = Some(Gen {
            chat_id,
            msg_id: prepared.msg_id,
            character_id: prepared.character_id,
            started: Instant::now(),
            content: prepared.start.clone(),
            cancel: cancel.clone(),
            kind: prepared.kind,
        });
        // An existing reply is written again or on: every device shows it as being written.
        if prepared.kind != Kind::Reply {
            match g.snapshot(chat_id) {
                Ok(Some(snapshot)) => drop(self.tx.send(snapshot)),
                Ok(None) => {}
                Err(e) => eprintln!("storage error: {e}"),
            }
        }
        // Unloaded: this message loads the model again and its reply waits for it.
        let woke = lock(&self.idle).load();
        drop(g);
        if woke {
            self.log.info("engine", "load", "model loaded again for a new message");
            self.woke();
        }
        self.log.write(
            Entry::new(Level::Info, "chat", "generate", "generation started").chat(chat_id).character(prepared.character_id).data(prepared.log.clone()),
        );
        let this = self.clone();
        tokio::spawn(async move {
            // A panic while writing ends this reply like an error, so the hub does not stay busy.
            let writing = std::panic::AssertUnwindSafe(this.generate(&prepared)).catch_unwind();
            let result = tokio::select! {
                _ = cancel.notified() => Ok((None, true)),
                r = writing => r.unwrap_or_else(|_| Err("generation panicked".to_string())).map(|t| (t, false)),
            };
            let (timings, stopped, error) = match result {
                Ok((t, stopped)) => (t, stopped, None),
                Err(e) => (None, true, Some(e)),
            };
            this.finish(timings, stopped, error);
        });
    }

    /// Persist the reply (or drop it if nothing was written) and tell everyone.
    fn finish(&self, timings: Option<Timings>, stopped: bool, error: Option<String>) {
        let mut g = lock(&self.inner);
        let Some(gen) = g.gen.take() else { return };
        lock(&self.idle).reply_done(Instant::now());
        let content = gen.content.trim();
        // `kept`: the chat holds new text of the model now.
        let (stored, kept) = match gen.kind {
            Kind::Reply if content.is_empty() => (g.store.delete_message(gen.msg_id), false),
            Kind::Reply => (g.store.finish_message(gen.msg_id, content, timings.as_ref()), true),
            // Nothing written (no model, stopped at once): the reply stays as it was.
            Kind::Variant if content.is_empty() => (Ok(()), false),
            Kind::Variant => (g.store.add_variant(gen.msg_id, content, timings.as_ref()).map(|_| ()), true),
            Kind::Continue { base } if content.len() <= base => (Ok(()), false),
            Kind::Continue { .. } => (g.store.continue_message(gen.msg_id, content, timings.as_ref()), true),
        };
        if let Err(e) = &stored {
            eprintln!("storage error: {e}");
        }
        let mut data = json!({
            "reply_message_id": gen.msg_id, "reply_chars": content.chars().count(),
            "ms": gen.started.elapsed().as_millis() as u64, "kept": kept, "mode": gen.kind.mode(),
        });
        if let Some(t) = &timings {
            data["tokens"] = json!(t.predicted);
            data["tok_s"] = json!((t.tok_s * 10.0).round() / 10.0);
            data["prompt_tokens"] = json!(t.processed);
            data["cached_tokens"] = json!(t.cached);
        }
        if let Err(e) = &stored {
            data["storage_error"] = json!(e.to_string());
        }
        let (level, message) = match (&error, stopped) {
            (Some(e), _) => (Level::Error, format!("generation failed: {e}")),
            (None, true) => (Level::Info, "generation stopped by the user".to_string()),
            (None, false) => (Level::Info, "generation finished".to_string()),
        };
        self.log.write(Entry::new(level, "chat", "generate", message).chat(gen.chat_id).character(gen.character_id).data(data));
        let _ = self.tx.send(
            json!({
                "type": "done", "chat_id": gen.chat_id, "id": gen.msg_id, "content": content,
                "removed": gen.kind == Kind::Reply && !kept, "stopped": stopped, "timings": timings, "error": error,
                "mode": gen.kind.mode(), "kept": kept,
            })
            .to_string(),
        );
        // A reply written again or on: its variants (or its old text, if nothing came) are in
        // the chat's new snapshot.
        if gen.kind != Kind::Reply {
            match g.snapshot(gen.chat_id) {
                Ok(Some(snapshot)) => drop(self.tx.send(snapshot)),
                Ok(None) => {}
                Err(e) => eprintln!("storage error: {e}"),
            }
            let _ = self.tx.send(json!({ "type": "library" }).to_string());
        }
    }

    /// Deletes a chat (undo: `restore_chat`). Err("busy"): the model is writing in it right now.
    pub fn delete_chat(&self, chat_id: i64) -> Result<Res<bool>, &'static str> {
        let mut g = lock(&self.inner);
        if g.writing_in(chat_id) {
            self.log.write(Entry::new(Level::Warn, "chat", "message", "chat delete refused: busy").chat(chat_id));
            return Err("busy");
        }
        Ok(g.store.set_chat_deleted(chat_id, true))
    }

    /// Restores the database from the backup file at `src`, after a `before-restore` backup of
    /// the current state (answered). Holds the lock throughout, so nothing is written in between,
    /// and the backups folder, so `src` is neither deleted nor pruned meanwhile.
    /// Err: `busy` (the model is writing), `bad_backup`, `backup_newer`, `storage`. The caller
    /// tells the devices (`library_changed`).
    pub fn restore_backup(&self, src: &std::path::Path) -> Result<crate::backup::Info, &'static str> {
        let mut g = lock(&self.inner);
        if g.gen.is_some() {
            return Err("busy");
        }
        let held = crate::backup::hold();
        crate::backup::check(src, store::schema_target())?;
        let (safety, _) = g.store.backup_before_restore(&held, src).map_err(storage_err)?;
        g.store.restore_from(src).map_err(storage_err)?;
        Ok(safety)
    }

    pub fn stop(&self) {
        if let Some(gen) = &lock(&self.inner).gen {
            self.log.write(Entry::new(Level::Info, "chat", "generate", "stop requested").chat(gen.chat_id));
            gen.cancel.notify_one();
        }
    }

    fn append_token(&self, text: &str) {
        let mut g = lock(&self.inner);
        if let Some(gen) = g.gen.as_mut() {
            gen.content.push_str(text);
            let _ = self
                .tx
                .send(json!({ "type": "token", "chat_id": gen.chat_id, "id": gen.msg_id, "text": text }).to_string());
        }
    }

    /// What the model server's chat template makes of `msgs`.
    async fn render(&self, msgs: &[ChatMsg], reasoning: bool) -> Result<String, String> {
        let tpl: Value = self
            .http
            .post(format!("{}/apply-template", self.llama_url()))
            .json(&json!({ "messages": msgs, "chat_template_kwargs": { "enable_thinking": reasoning } }))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("apply-template failed: {e}"))?
            .json()
            .await
            .map_err(|e| format!("apply-template: bad JSON: {e}"))?;
        Ok(tpl["prompt"].as_str().ok_or("apply-template: no prompt in response")?.to_string())
    }

    /// How many tokens the model makes of `text` (llama-server's `/tokenize`). None: it cannot
    /// say (an older build, another server).
    async fn tokenize(&self, text: &str) -> Option<usize> {
        let req = self.http.post(format!("{}/tokenize", self.llama_url())).timeout(Duration::from_secs(30));
        let v: Value = req.json(&json!({ "content": text, "add_special": false })).send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
        v["tokens"].as_array().map(Vec::len)
    }

    /// Tokens of `text`: counted by the model server, else estimated. `exact` turns false once
    /// the model server could not count (the rest of this prompt is then estimated, too).
    async fn count(&self, text: &str, exact: &mut bool) -> usize {
        if *exact {
            match self.tokenize(text).await {
                Some(n) => return n,
                None => *exact = false,
            }
        }
        estimate_tokens(text)
    }

    /// Tokens of the turns' texts, for `model`: from the cache, else counted (a few at a time)
    /// and kept. Estimated when the model server cannot count.
    async fn count_turns(&self, model: &str, turns: &[Turn], exact: &mut bool) -> Vec<usize> {
        use std::hash::{Hash, Hasher};
        let key = |text: &str| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut h);
            h.finish()
        };
        let known: Vec<Option<usize>> = {
            let mut cache = lock(&self.tokens);
            if cache.model != model || cache.counts.len() > TOKEN_CACHE_MAX {
                *cache = TokenCache { model: model.to_string(), counts: Default::default() };
            }
            turns.iter().map(|t| cache.counts.get(&key(&t.content)).copied()).collect()
        };
        let missing: Vec<&str> = turns.iter().zip(&known).filter(|(_, k)| k.is_none()).map(|(t, _)| t.content.as_str()).collect();
        let counted: Vec<Option<usize>> = if *exact {
            let requests: Vec<_> = missing.iter().map(|text| self.tokenize(text)).collect();
            futures_util::stream::iter(requests).buffered(8).collect().await
        } else {
            vec![None; missing.len()]
        };
        if counted.iter().any(Option::is_none) {
            *exact = false;
        }
        let mut cache = lock(&self.tokens);
        let mut counted = missing.iter().zip(counted);
        known
            .into_iter()
            .map(|k| {
                k.unwrap_or_else(|| {
                    let (text, n) = counted.next().expect("one count per missing text");
                    match n {
                        Some(n) => {
                            if cache.model == model {
                                cache.counts.insert(key(text), n);
                            }
                            n
                        }
                        None => estimate_tokens(text),
                    }
                })
            })
            .collect()
    }

    /// The prompt for `p` that fits into the model's context (layout B, `docs/cache.md` rule 3):
    /// the whole history if it fits, else the history cut by `prompt::cut` — the first message,
    /// then the newest turns, the oldest ones in between left out in blocks of a quarter of the
    /// room, so the prompt stays the same at its start from one reply to the next and the model
    /// server reads it from its cache. Err(`context_full`): not even the head and the newest
    /// message fit. The context comes from the model server (`/props`), else from the profile's
    /// `-c`; unknown, the prompt goes as it is.
    async fn fit(&self, p: &Prepared) -> Result<Fitted, String> {
        let msgs = p.source.build(Cut::NONE);
        let rendered = self.render(&msgs, p.reasoning).await?;
        let props = self.props().await;
        let (model, ctx, _) = props.as_ref().map(crate::engine::model::from_props).unwrap_or_default();
        let Some(n_ctx) = ctx.or(p.ctx_hint) else { return Ok(Fitted { rendered }) };
        let budget = prompt_budget(n_ctx, p.sampler.n_predict, p.reasoning);
        // Every token stands for at least one byte of text: fewer bytes than the budget fit.
        if rendered.len() + p.start.len() <= budget {
            return Ok(Fitted { rendered });
        }
        let model = model.map(|m| m.display().to_string()).unwrap_or_default();
        let mut exact = true;
        let start = if p.start.is_empty() { 0 } else { self.count(&p.start, &mut exact).await };
        let frame = self.render(&p.source.frame(), p.reasoning).await?;
        let fixed = self.count(&frame, &mut exact).await + start;
        // `newest`: what the newest message costs (None: the head alone is too big).
        let full = |newest: Option<usize>, exact: bool| {
            self.log.write(
                Entry::new(Level::Warn, "chat", "generate", "prompt does not fit the context")
                    .chat(p.chat_id)
                    .data(json!({ "n_ctx": n_ctx, "budget": budget, "fixed_tokens": fixed, "newest_tokens": newest, "exact": exact })),
            );
            Err(CONTEXT_FULL.to_string())
        };
        if fixed >= budget {
            return full(None, exact);
        }
        let sizes: Vec<usize> = self.count_turns(&model, &p.source.turns, &mut exact).await.into_iter().map(|n| n + prompt::TURN_OVERHEAD).collect();
        let mut room = budget - fixed;
        // The cut is planned with counts per message; the prompt the template renders from it is
        // counted once more as a whole. Over the budget after all: plan with less room.
        for _ in 0..4 {
            let Some(cut) = prompt::cut(&sizes, room) else { return full(sizes.last().copied(), exact) };
            let rendered = if cut == Cut::NONE {
                rendered.clone()
            } else {
                let msgs = p.source.build(cut);
                self.render(&msgs, p.reasoning).await?
            };
            let total = self.count(&rendered, &mut exact).await + start;
            if total <= budget {
                if cut != Cut::NONE {
                    let turns = p.source.turns.len();
                    self.log.write(Entry::new(Level::Info, "chat", "generate", "history shortened to fit the context").chat(p.chat_id).data(json!({
                        "n_ctx": n_ctx, "budget": budget, "prompt_tokens": total, "dropped_turns": cut.dropped(turns),
                        "kept_turns": turns - cut.dropped(turns), "kept_first": cut.keep_first, "exact": exact,
                    })));
                }
                return Ok(Fitted { rendered });
            }
            room = room.saturating_sub(total - budget + prompt::TURN_OVERHEAD);
        }
        full(sizes.last().copied(), exact)
    }

    /// Dropping this future (on stop) drops the upstream connection, which aborts llama-server's job.
    async fn generate(&self, p: &Prepared) -> Result<Option<Timings>, String> {
        self.wait_ready().await;
        let Fitted { rendered, .. } = self.fit(p).await?;
        // A reply that is written on stays the open turn of the character: its text follows the
        // prompt it was written from, so the model goes on from its last word (and reads that
        // prompt from its cache).
        let prompt = format!("{rendered}{}", p.start);

        // Sampler of the profile (temperature always set, n_predict as the hard cap: the answer
        // length itself, wider with reasoning), then what the app needs. Timings come with every
        // token: they count the reply's tokens and stay the reply's timings when it is ended at a
        // sentence end.
        let mut body = p.sampler.request_fields(p.reasoning);
        body.insert("prompt".into(), prompt.into());
        body.insert("stream".into(), true.into());
        body.insert("cache_prompt".into(), true.into());
        body.insert("stop".into(), json!(p.stop));
        body.insert("timings_per_token".into(), true.into());
        let resp = self
            .http
            .post(format!("{}/completion", self.llama_url()))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("completion failed: {e}"))?;
        // Longer than the context after all (the model server's context was not known, or it
        // counts differently): the same clear code as when the prompt cannot be cut to fit.
        if resp.status() == reqwest::StatusCode::BAD_REQUEST {
            let body = resp.text().await.unwrap_or_default();
            if exceeds_context(&body) {
                return Err(CONTEXT_FULL.to_string());
            }
            return Err("completion failed: HTTP status client error (400 Bad Request)".to_string());
        }
        let resp = resp.error_for_status().map_err(|e| format!("completion failed: {e}"))?;

        // The answer length is never exceeded: from 85 % of it the reply ends at the first clean
        // sentence end (no speech or action open), and one that reaches the hard cap (the length
        // itself) is cut back to a sentence end (a reply written on counts its own part only and
        // knows what its text left open). A reasoning model writes its thinking into the same
        // stream, which must not be ended mid-way, so its reply only has the wider hard cap.
        let mut soft = (!p.reasoning).then(|| SoftStop::new(p.sampler.reply_length()).with_base(&p.start));
        let mut stream = resp.bytes_stream();
        let mut lines = LineBuf::default();
        let mut timings = None;
        let mut tokens = 0u64;
        'stream: while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("stream error: {e}"))?;
            for line in lines.push(&chunk) {
                let Some(v) = parse_sse_line(&line) else { continue };
                let c = read_chunk(&v);
                // Without per-token timings (an older model server) every piece counts as a token.
                tokens = c.timings.as_ref().map_or(tokens + 1, |t| t.predicted.max(tokens));
                if c.timings.is_some() {
                    timings = c.timings;
                }
                if c.content.is_empty() {
                    continue;
                }
                match soft.as_mut().map(|s| s.push(&c.content, tokens)) {
                    None => self.append_token(&c.content),
                    Some(Step::Go(text)) => self.append_text(&text),
                    Some(Step::End(text)) => {
                        self.append_text(&text);
                        // Dropping the stream ends the model server's job.
                        soft = None;
                        break 'stream;
                    }
                }
            }
        }
        // The stream ended (by itself or at the hard cap): the held back text is cut back to a
        // sentence end and closed, or all of it goes on if the reply stayed under 85 %.
        if let Some(s) = soft.as_mut() {
            self.append_text(&s.finish());
        }
        Ok(timings)
    }

    /// `append_token` for text that may be empty (the soft stop holds pieces back).
    fn append_text(&self, text: &str) {
        if !text.is_empty() {
            self.append_token(text);
        }
    }
}

// ---------------------------------------------------------------- SSE parsing

/// Splits a byte stream into lines, tolerating chunk boundaries anywhere (even inside UTF-8).
#[derive(Default)]
pub struct LineBuf(Vec<u8>);

impl LineBuf {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.0.extend_from_slice(bytes);
        let mut out = vec![];
        while let Some(pos) = self.0.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.0.drain(..=pos).collect();
            out.push(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string());
        }
        out
    }
}

/// `data: {json}` -> parsed JSON; anything else (comments, blanks, `[DONE]`) -> None.
pub fn parse_sse_line(line: &str) -> Option<Value> {
    let data = line.strip_prefix("data:")?.trim();
    serde_json::from_str(data).ok()
}

#[derive(Debug, PartialEq)]
pub struct Chunk {
    pub content: String,
    pub stop: bool,
    pub timings: Option<Timings>,
}

pub fn read_chunk(v: &Value) -> Chunk {
    let t = &v["timings"];
    let timings = t.is_object().then(|| Timings {
        tok_s: t["predicted_per_second"].as_f64().unwrap_or(0.0),
        processed: t["prompt_n"].as_u64().unwrap_or(0),
        cached: t["cache_n"].as_u64().unwrap_or(0),
        predicted: t["predicted_n"].as_u64().unwrap_or(0),
    });
    Chunk {
        content: v["content"].as_str().unwrap_or("").to_string(),
        stop: v["stop"].as_bool().unwrap_or(false),
        timings,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::store::tests::{sample, temp_store};
    use rp_core::prompt::Priority;

    #[test]
    fn lines_across_chunks() {
        let mut b = LineBuf::default();
        assert!(b.push(b"data: {\"content\":\"a").is_empty());
        let l = b.push(b"\"}\n\ndata: {\"content\":\"b\"}\n");
        assert_eq!(l, vec!["data: {\"content\":\"a\"}", "", "data: {\"content\":\"b\"}"]);
    }

    #[test]
    fn utf8_split_inside_char() {
        let full = "data: {\"content\":\"ä\"}\n".as_bytes();
        let cut = full.iter().position(|&b| b == 0xC3).unwrap() + 1;
        let mut b = LineBuf::default();
        assert!(b.push(&full[..cut]).is_empty());
        let l = b.push(&full[cut..]);
        assert_eq!(parse_sse_line(&l[0]).unwrap()["content"], "ä");
    }

    #[test]
    fn sse_line_parsing() {
        assert!(parse_sse_line("").is_none());
        assert!(parse_sse_line(": keepalive").is_none());
        assert!(parse_sse_line("data: [DONE]").is_none());
        assert_eq!(parse_sse_line("data: {\"content\":\"x\"}").unwrap()["content"], "x");
    }

    #[test]
    fn final_chunk_has_timings() {
        let v: Value = serde_json::from_str(
            r#"{"content":"","stop":true,"timings":{"prompt_n":12,"cache_n":30,"predicted_n":20,"predicted_per_second":95.5}}"#,
        )
        .unwrap();
        let c = read_chunk(&v);
        assert!(c.stop);
        assert_eq!(c.timings, Some(Timings { tok_s: 95.5, processed: 12, cached: 30, predicted: 20 }));
        assert!(read_chunk(&json!({"content":"hi","stop":false})).timings.is_none());
    }

    #[test]
    fn open_chat_reuses_or_creates_with_greeting() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Evening, {{user}}."}"#), None).unwrap().id;
        store.set_user_name("Lukas").unwrap();
        let hub = Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir));

        let first = hub.open_chat(id, false).unwrap().unwrap();
        assert_eq!(hub.open_chat(id, false).unwrap(), Some(first), "reuses the latest chat");
        let second = hub.open_chat(id, true).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(hub.open_chat(999, false).unwrap(), None);

        let (snapshot, _rx) = hub.open_view(second).unwrap().unwrap();
        let v: Value = serde_json::from_str(&snapshot).unwrap();
        assert_eq!(v["messages"][0]["content"], "Evening, Lukas.");
        assert_eq!(v["character"]["name"], "Mara");
        assert_eq!(v["chat"]["messages"], 1);
        assert_eq!(v["busy"], false);
        assert!(hub.open_view(999).unwrap().is_none());
        drop(hub);
        let _ = std::fs::remove_dir_all(dir); // the log writer may still close its file
    }

    #[test]
    fn message_changes_reach_every_device() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (first, second) = hub.store(|s| {
            (s.add_message(chat, "user", "Hello").unwrap().id, s.add_message(chat, "assistant", "Welcome").unwrap().id)
        });
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        // The next `history` event: (texts, message count of the chat).
        let mut next = move || loop {
            let ev: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
            if ev["type"] == "history" {
                let texts: Vec<String> =
                    ev["messages"].as_array().unwrap().iter().map(|m| m["content"].as_str().unwrap().to_string()).collect();
                return (texts, ev["chat"]["messages"].as_i64().unwrap());
            }
        };

        assert_eq!(hub.edit_message(first, "  "), Err("empty"));
        assert_eq!(hub.edit_message(999, "x"), Err("message_missing"));
        hub.edit_message(first, " Hello there ").unwrap();
        assert_eq!(next(), (vec!["Hi.".into(), "Hello there".into(), "Welcome".into()], 3));

        let fork = hub.fork(first, "Branch").unwrap();
        assert_ne!(fork, chat);
        assert_eq!(hub.store(|s| s.messages(fork)).unwrap().len(), 2);

        assert_eq!(hub.rewind(first), Ok((chat, 2)));
        assert_eq!(next(), (vec!["Hi.".into()], 1));
        assert_eq!(hub.rewind(second), Err("message_missing"), "it is gone with the rewind");
        hub.undo_rewind(chat).unwrap();
        assert_eq!(next().1, 3);
        assert_eq!(hub.undo_rewind(chat), Err("undo_gone"));
        assert_eq!(hub.undo_rewind(999), Err("chat_missing"));
        drop(hub);
        let _ = std::fs::remove_dir_all(dir); // the log writer may still close its file
    }

    /// While the model writes in a chat, its messages cannot be changed or branched off.
    #[tokio::test]
    async fn no_message_changes_while_the_model_writes() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let greeting = hub.store(|s| s.messages(chat)).unwrap()[0].id;
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();

        hub.send(chat, "Hello".into()).unwrap(); // the reply task has not run yet
        assert_eq!(hub.edit_message(greeting, "x"), Err("busy"));
        assert_eq!(hub.rewind(greeting), Err("busy"));
        assert_eq!(hub.fork(greeting, ""), Err("busy"));
        assert_eq!(hub.undo_rewind(chat), Err("busy"));
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        assert!(hub.edit_message(greeting, "Hi again.").is_ok(), "free once the reply ended");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A chat imported with reply variants: the user's message has none, the reply three.
    fn swiped_chat(hub: &Hub, character: i64) -> (i64, i64, i64) {
        let msg = |role, content: &str, swipes: Option<(Vec<String>, usize)>| store::ImportedMsg {
            role,
            content: content.into(),
            created_at: 100,
            swipes,
        };
        let variants = vec!["Nach Amrum.".to_string(), "Nach Föhr.".into(), "Kaffee.".into()];
        let imported = store::ImportedChat {
            title: "Fähre".into(),
            created_at: 100,
            hash: "swipes".into(),
            messages: vec![msg("user", "Wohin?", None), msg("assistant", "Nach Amrum.", Some((variants, 0)))],
        };
        hub.store(|s| {
            let chat = s.import_chat(character, &imported).unwrap().0;
            let m = s.messages(chat).unwrap();
            (chat, m[0].id, m[1].id)
        })
    }

    /// Choosing a variant changes the message's text and active index for every device, survives
    /// a restart, and refuses indexes outside the list and messages without variants.
    #[test]
    fn choosing_a_reply_variant() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Theo","first_mes":"Moin."}"#), None).unwrap().id;
        let hub = Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir));
        let (chat, user_msg, reply) = swiped_chat(&hub, id);
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();

        assert_eq!(hub.set_swipe(reply, 3), Err("swipe_out_of_range"));
        assert_eq!(hub.set_swipe(reply, -1), Err("swipe_out_of_range"));
        assert_eq!(hub.set_swipe(user_msg, 0), Err("no_swipes"));
        assert_eq!(hub.set_swipe(999, 0), Err("message_missing"));
        assert!(rx.try_recv().is_err(), "a refused choice sends nothing");

        hub.set_swipe(reply, 1).unwrap();
        let history = loop {
            let ev: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
            if ev["type"] == "history" {
                break ev;
            }
        };
        let last = &history["messages"][1];
        assert_eq!((last["content"].as_str(), last["swipe_id"].as_u64()), (Some("Nach Föhr."), Some(1)));
        assert_eq!(last["swipes"].as_array().map(Vec::len), Some(3), "the variants stay");
        drop(hub);

        let reopened = Store::open(&dir).unwrap();
        let m = reopened.messages(chat).unwrap();
        assert_eq!((m[1].content.as_str(), m[1].swipe_id, m[1].swipes.as_ref().map(Vec::len)), ("Nach Föhr.", Some(1), Some(3)));
        assert_eq!((m[0].content.as_str(), m[0].swipes.clone()), ("Wohin?", None));
        drop(reopened);
        let _ = std::fs::remove_dir_all(dir); // the log writer may still close its file
    }

    /// While the model writes in a chat, no variant of its messages can be chosen.
    #[tokio::test]
    async fn no_variant_choice_while_the_model_writes() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Theo","first_mes":"Moin."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir)));
        let (chat, _, reply) = swiped_chat(&hub, id);
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();

        hub.send(chat, "Und dann?".into()).unwrap(); // the reply task has not run yet
        assert_eq!(hub.set_swipe(reply, 2), Err("busy"));
        assert_eq!(hub.store(|s| s.messages(chat)).unwrap()[1].content, "Nach Amrum.", "unchanged");
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        assert!(hub.set_swipe(reply, 2).is_ok(), "free once the reply ended");
        assert_eq!(hub.store(|s| s.messages(chat)).unwrap()[1].content, "Kaffee.");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Full send path against a model server that is not there: the user message stays, the
    /// empty reply is removed again, and the hub is free for the next attempt.
    #[tokio::test]
    async fn failed_generation_leaves_a_clean_chat() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();

        assert_eq!(hub.send(chat, "  ".into()), Err("empty"));
        assert_eq!(hub.send(999, "x".into()), Err("chat_missing"));
        hub.send(chat, "Hello".into()).unwrap();
        assert_eq!(hub.send(chat, "again".into()), Err("busy"));

        let mut kinds = vec![];
        loop {
            let ev: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            kinds.push(ev["type"].as_str().unwrap().to_string());
            if ev["type"] == "done" {
                assert_eq!(ev["removed"], true);
                assert!(ev["error"].as_str().unwrap().contains("apply-template failed"));
                break;
            }
        }
        assert_eq!(kinds, ["message", "assistant_start", "done"]);
        let roles: Vec<String> = hub.store(|s| s.messages(chat)).unwrap().into_iter().map(|m| m.role).collect();
        assert_eq!(roles, ["assistant", "user"]);
        assert!(hub.send(chat, "retry".into()).is_ok(), "hub is free again");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A chat bound to a persona keeps playing it (name as `{{user}}`, fields in the head) after
    /// the active persona changes; a free chat follows the switch.
    #[test]
    fn a_bound_chat_keeps_its_persona_in_the_prompt() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Evening, {{user}}."}"#), None).unwrap().id;
        let bea = store
            .add_persona(&store::PersonaEdit {
                pronouns: Some("sie/ihr".into()),
                appearance: Some("Narbe über dem Auge.".into()),
                ..store::PersonaEdit::name("Bea")
            })
            .unwrap()
            .id;
        let carl = store.add_persona(&store::PersonaEdit { description: Some("{{user}} ist Koch.".into()), ..store::PersonaEdit::name("Carl") }).unwrap().id;
        store.set_default_persona(id, Some(bea)).unwrap();
        let hub = Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir));

        let bound = hub.open_chat(id, true).unwrap().unwrap();
        hub.store(|s| s.set_default_persona(id, None)).unwrap();
        let free = hub.open_chat(id, true).unwrap().unwrap();
        hub.store(|s| s.activate_persona(carl)).unwrap();
        let prompt = |chat| hub.inner.lock().unwrap().prompt(chat, None).unwrap().unwrap();

        let (msgs, name) = prompt(bound);
        assert_eq!(name, "Bea");
        assert_eq!(msgs[1].content, "(Die Szene beginnt.)");
        assert_eq!(msgs[2].content, "Evening, Bea.", "the greeting greets the bound persona");
        assert!(msgs[0].content.contains("Der Nutzer spielt Bea."));
        assert!(msgs[0].content.contains("# Nutzerfigur: Bea (sie/ihr) (gespielt vom Nutzer)\n\nAussehen: Narbe über dem Auge."));
        assert!(!msgs[0].content.contains("Carl"));

        let (msgs, name) = prompt(free);
        assert_eq!(name, "Carl");
        assert!(msgs[0].content.contains("# Nutzerfigur: Carl (gespielt vom Nutzer)\n\nCarl ist Koch."));
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The chat's prompt follows the setting `language`: German leaves it as it was, any other
    /// language stands once in the head and once at the end of the user's last message, from the
    /// next reply on. The fixed rule stays in every one.
    #[test]
    fn the_prompt_asks_for_the_apps_language() {
        use rp_core::prompt::Language;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir));
        let chat = hub.open_chat(id, true).unwrap().unwrap();
        hub.store(|s| s.add_message(chat, "user", "Guten Abend.")).unwrap();
        let prompt = || hub.inner.lock().unwrap().prompt(chat, None).unwrap().unwrap().0;

        assert_eq!(hub.store(|s| s.language()).unwrap(), Language::De, "German until one is chosen");
        let german = prompt();
        assert!(german[0].content.contains("- Schreibe in der Sprache, in der der Nutzer schreibt (Standard: Deutsch)."));
        // No language note for German, only the answer length (default 800 tokens).
        let length = |lang| rp_core::prompt::length_note(lang, DEFAULT_N_PREDICT).unwrap();
        assert_eq!(german.last().unwrap().content, format!("Guten Abend.\n\n[Anweisung für diese Antwort: {}]", length(Language::De)));

        for (lang, in_head, note) in [
            (Language::En, "Always write your replies in English.", "Reply in English."),
            (Language::Es, "Escribe siempre tus respuestas en español.", "Responde en español."),
            (Language::Ru, "Всегда пиши свои ответы на русском языке.", "Отвечай на русском языке."),
        ] {
            hub.store(|s| s.set_language(lang)).unwrap();
            let msgs = prompt();
            assert!(msgs[0].content.contains(in_head) && !msgs[0].content.contains("Standard: Deutsch"), "{lang:?}");
            assert_eq!(msgs.last().unwrap().content, format!("Guten Abend.\n\n[Anweisung für diese Antwort: {note}\n{}]", length(lang)));
            assert_eq!(msgs[1..msgs.len() - 1], german[1..german.len() - 1], "the history is the same in every language");
        }
        hub.store(|s| s.set_language(Language::De)).unwrap();
        assert_eq!(prompt(), german, "back to German: the prompt as before");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The priority sentence in the prompt follows the character's override, else the setting of
    /// the chat's persona.
    #[test]
    fn the_prompt_uses_the_effective_priority() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","description":"{{user}} ist ihr Ehemann.","first_mes":"Hi."}"#), None).unwrap().id;
        let bea = store.add_persona(&store::PersonaEdit { pronouns: Some("sie/ihr".into()), ..store::PersonaEdit::name("Bea") }).unwrap().id;
        store.activate_persona(bea).unwrap();
        let hub = Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir));
        let chat = hub.open_chat(id, true).unwrap().unwrap();
        let head = || hub.inner.lock().unwrap().prompt(chat, None).unwrap().unwrap().0[0].content.clone();
        let persona_first = "Vorrang: Beschreibt die Figurenkarte Bea anders, gelten für Name, Aussehen, Persönlichkeit und Sprechweise die Angaben hier; Rolle, Beziehung und Lage in der Geschichte kommen aus der Karte.";
        let card_first = "Vorrang: Beschreibt die Figurenkarte Bea anders, gilt die Karte; die Angaben hier ergänzen nur, was die Karte offenlässt.";
        let ends = |h: String, sentence: &str| assert!(h.contains(&format!("(sie/ihr) (gespielt vom Nutzer)\n\n{sentence}")), "{h}");

        ends(head(), persona_first);
        let to_card = store::PersonaEdit { card_priority: Some("card".into()), ..store::PersonaEdit::default() };
        hub.store(|s| s.update_persona(bea, &to_card)).unwrap().unwrap();
        ends(head(), card_first);
        assert!(hub.store(|s| s.set_user_priority(id, Some(Priority::Persona))).unwrap());
        ends(head(), persona_first);
        assert!(hub.store(|s| s.set_user_priority(id, None)).unwrap());
        ends(head(), card_first);
        assert_eq!(head().matches("Vorrang:").count(), 1);
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A user message stores the chat's persona; binding a chat sends its new snapshot with the
    /// persona ids of the chat and of every message.
    #[tokio::test]
    async fn user_messages_remember_their_persona() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let user = store.active_persona_id().unwrap();
        let bea = store.add_persona(&store::PersonaEdit::name("Bea")).unwrap().id;
        let hub = Arc::new(Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();

        hub.send(chat, "Als User".into()).unwrap();
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        assert_eq!(hub.set_chat_persona(chat, Some(bea)).unwrap(), store::Bind::Done);
        let snapshot: Value = loop {
            let ev: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            if ev["type"] == "history" {
                break ev;
            }
        };
        assert_eq!((snapshot["chat"]["persona_id"].as_i64(), snapshot["chat"]["effective_persona_id"].as_i64()), (Some(bea), Some(bea)));
        hub.send(chat, "Als Bea".into()).unwrap();
        while !rx.recv().await.unwrap().contains("\"done\"") {}

        let (snapshot, _rx) = hub.open_view(chat).unwrap().unwrap();
        let v: Value = serde_json::from_str(&snapshot).unwrap();
        let ids: Vec<Value> = v["messages"].as_array().unwrap().iter().map(|m| m["persona_id"].clone()).collect();
        assert_eq!(ids, [Value::Null, json!(user), json!(bea)], "greeting null, then each user message with its persona");
        assert_eq!(hub.set_chat_persona(chat, Some(999)).unwrap(), store::Bind::NoPersona);
        assert_eq!(hub.set_chat_persona(999, None).unwrap(), store::Bind::NoTarget);
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Generations and message changes are logged with ids and sizes, never with their text.
    #[tokio::test]
    async fn the_log_tells_what_happened_without_any_text() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        hub.send(chat, "Geheimer Text 4711".into()).unwrap();
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        let user_msg = hub.store(|s| s.messages(chat)).unwrap()[1].id;
        hub.edit_message(user_msg, "Noch geheimer 0815").unwrap();
        hub.log().flush(std::time::Duration::from_secs(5));

        let conn = crate::logs::open_read(&dir).unwrap();
        let page = crate::logs::query(&conn, &crate::logs::Query::default()).unwrap();
        let all = serde_json::to_string(&page.entries).unwrap();
        let messages: Vec<&str> = page.entries.iter().map(|e| e.message.as_str()).collect();
        assert!(messages.contains(&"generation started"));
        assert!(messages.iter().any(|m| m.starts_with("generation failed: apply-template failed")));
        assert!(messages.contains(&"message edited"));
        let failed = page.entries.iter().find(|e| e.level == "error").unwrap();
        assert_eq!((failed.chat_id, failed.character_id), (Some(chat), Some(id)));
        assert!(all.contains("\"user_chars\":18"));
        assert!(!all.contains("Geheim") && !all.contains("geheim") && !all.contains("Hi."), "no message text in the log");
        drop(conn);
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What a ChatML chat template renders from an `/apply-template` request; `system: false` is a
    /// template that leaves the system message out.
    pub(crate) fn render(request: &Value, system: bool) -> String {
        let msgs = request["messages"].as_array().cloned().unwrap_or_default();
        msgs.iter()
            .filter(|m| system || m["role"] != "system")
            .map(|m| format!("<|im_start|>{}\n{}<|im_end|>\n", m["role"].as_str().unwrap_or(""), m["content"].as_str().unwrap_or("")))
            .collect::<String>()
            + "<|im_start|>assistant\n"
    }

    /// A real local HTTP server in llama-server's place: answers `/apply-template` and streams a
    /// short `/completion`, and keeps every request body it got.
    async fn llama_receiver() -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
        llama_template(true).await
    }

    /// [`llama_receiver`] with a chat template that keeps (`system`) or drops the system message.
    async fn llama_template(system: bool) -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
        use axum::routing::post;
        let got: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let (a, b) = (got.clone(), got.clone());
        let app = axum::Router::new()
            .route(
                "/apply-template",
                post(move |axum::Json(v): axum::Json<Value>| async move {
                    let prompt = render(&v, system);
                    a.lock().unwrap().push(("apply-template".into(), v));
                    axum::Json(json!({ "prompt": prompt }))
                }),
            )
            .route(
                "/completion",
                post(move |axum::Json(v): axum::Json<Value>| async move {
                    b.lock().unwrap().push(("completion".into(), v));
                    concat!(
                        "data: {\"content\":\"Hallo\",\"stop\":false}\n\n",
                        "data: {\"content\":\"\",\"stop\":true,\"timings\":{\"prompt_n\":5,\"cache_n\":0,\"predicted_n\":1,\"predicted_per_second\":12.5}}\n\n"
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, got)
    }

    /// Unloaded: a message loads the model again (every device hears "waking"), its reply waits
    /// until the model is ready and then comes as usual; unloading is refused while it is written.
    /// The quiet time unloads only once a reply is done.
    #[tokio::test]
    async fn a_message_loads_an_unloaded_model_and_waits_for_it() {
        let (url, got) = llama_receiver().await;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        // 200 ms per "minute": 5 minutes = 1 s, long enough that a busy machine (parallel builds)
        // does not run out the quiet time between the finished reply and the first check.
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)).with_idle_minute(Duration::from_millis(200)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        hub.engine_event("ready", None, None);
        hub.unload().unwrap();
        assert!(hub.unloaded() && hub.idle_check(0));

        hub.send(chat, "Hallo".into()).unwrap();
        assert!(!hub.unloaded(), "the message wants the model again");
        assert_eq!(hub.unload(), Err("busy"), "never unload while a reply is written");
        let mut waking = false;
        while let Ok(Ok(raw)) = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
            let ev: Value = serde_json::from_str(&raw).unwrap();
            waking |= ev["type"] == "engine" && ev["event"] == "waking";
            assert_ne!(ev["type"], "done", "the reply waits for the model");
        }
        assert!(waking);
        assert!(got.lock().unwrap().is_empty(), "nothing goes to the model before it is ready");

        hub.engine_event("ready", None, None); // what the supervisor says once /health answers
        loop {
            let ev: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            if ev["type"] == "done" {
                assert_eq!((ev["content"].as_str(), ev["error"].clone()), (Some("Hallo"), Value::Null));
                break;
            }
        }
        assert!(!hub.idle_check(5), "the quiet time starts with the finished reply");
        tokio::time::sleep(Duration::from_millis(1250)).await;
        assert!(hub.idle_check(5), "5 (shortened) minutes later the model is unloaded");
        assert!(hub.load() && !hub.load(), "the button loads it once");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every request to the model server carries the sampler of the active profile at send
    /// time; without a profile it carries the old fixed values (temperature 0.8, n_predict 800).
    #[tokio::test]
    async fn requests_carry_the_sampler_of_the_active_profile() {
        let (url, got) = llama_receiver().await;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        let mut say = async |text: &str| {
            hub.send(chat, text.into()).unwrap();
            loop {
                let ev: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
                if ev["type"] == "done" {
                    assert_eq!((ev["content"].as_str(), ev["error"].clone()), (Some("Hallo"), Value::Null));
                    break;
                }
            }
            let all = got.lock().unwrap();
            let completion = all.iter().rev().find(|(path, _)| path == "completion").unwrap().1.clone();
            let template = all.iter().rev().find(|(path, _)| path == "apply-template").unwrap().1.clone();
            (completion, template)
        };

        // n_predict goes as the hard cap: the answer length itself (800), wider only with reasoning.
        let (c, t) = say("Erste").await;
        assert_eq!((c["temperature"].as_f64(), c["n_predict"].as_i64()), (Some(0.8), Some(800)), "no profile: the old length");
        assert_eq!(c["timings_per_token"], true);
        assert_eq!((c["prompt"].as_str(), c["stream"].as_bool(), c["cache_prompt"].as_bool()), (Some(render(&t, true).as_str()), Some(true), Some(true)));
        assert!(c.get("min_p").is_none() && c["stop"].is_array());
        assert_eq!(t["chat_template_kwargs"]["enable_thinking"], false);

        let cfg: crate::engine::EngineConfig = serde_json::from_str(r#"{ "llama_server": "x.exe", "model": "m.gguf" }"#).unwrap();
        let (_, profile) = hub.store(|s| s.import_engine_json(&cfg)).unwrap().unwrap();
        let (c, _) = say("Zweite").await;
        assert_eq!((c["temperature"].as_f64(), c["n_predict"].as_i64()), (Some(0.8), Some(800)), "the imported profile keeps the old values");

        let edit = store::ProfileEdit {
            sampler: Some(Sampler { temperature: Some(1.15), min_p: Some(0.05), top_k: Some(0), n_predict: Some(300), ..Sampler::default() }),
            reasoning: Some("on".into()),
            ..store::ProfileEdit::default()
        };
        hub.store(|s| s.update_profile(profile, &edit)).unwrap().unwrap();
        let (c, t) = say("Dritte").await;
        assert_eq!(
            (c["temperature"].as_f64(), c["min_p"].as_f64(), c["top_k"].as_i64(), c["n_predict"].as_i64()),
            (Some(1.15), Some(0.05), Some(0), Some(450)),
            "reasoning on: the wider cap"
        );
        assert_eq!(t["chat_template_kwargs"]["enable_thinking"], true);
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Two sentences, then more, one token each.
    const WORDS: [&str; 14] = [" Sie", " geht", " los", ".", " Dann", " bleibt", " sie", " stehen", ".", " Und", " sieht", " sich", " um", "."];
    /// A reply with speech in it, one token each (the speech ends with token 12).
    const SPEECH: [&str; 16] = [" Sie", " sagt", ":", " „", "Geh", " weg", ".", " Sofort", " sonst", " gehe", " ich", ".“", " Dann", " geht", " sie", "."];

    /// A model server that writes `words` one token per piece, with timings at every token like
    /// llama-server with `timings_per_token`, and stops after `n_predict` tokens like it does.
    async fn llama_words(words: &'static [&'static str]) -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
        use axum::routing::post;
        let got: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let (a, b) = (got.clone(), got.clone());
        let app = axum::Router::new()
            .route(
                "/apply-template",
                post(move |axum::Json(v): axum::Json<Value>| async move {
                    let prompt = render(&v, true);
                    a.lock().unwrap().push(("apply-template".into(), v));
                    axum::Json(json!({ "prompt": prompt }))
                }),
            )
            .route(
                "/completion",
                post(move |axum::Json(v): axum::Json<Value>| async move {
                    let cap = v["n_predict"].as_u64().unwrap_or(u64::MAX) as usize;
                    b.lock().unwrap().push(("completion".into(), v));
                    let n = cap.min(words.len());
                    let timings = |i: usize| json!({ "prompt_n": 3, "cache_n": 30, "predicted_n": i, "predicted_per_second": 20.0 });
                    let mut body: String = words[..n]
                        .iter()
                        .enumerate()
                        .map(|(i, w)| format!("data: {}\n\n", json!({ "content": w, "stop": false, "timings": timings(i + 1) })))
                        .collect();
                    body.push_str(&format!("data: {}\n\n", json!({ "content": "", "stop": true, "timings": timings(n) })));
                    body
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, got)
    }

    /// Waits for `done` and returns it with every streamed piece.
    async fn written(rx: &mut broadcast::Receiver<String>) -> (Value, String) {
        let mut seen = String::new();
        loop {
            let ev: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            match ev["type"].as_str() {
                Some("token") => seen.push_str(ev["text"].as_str().unwrap()),
                Some("done") => return (ev, seen),
                _ => {}
            }
        }
    }

    /// The answer length is never exceeded: the prompt asks for it at the end of the last user
    /// message, the model server gets the length itself as its hard cap (reasoning off), from 85 %
    /// of it the reply ends at the first sentence end (never mid-sentence, and the devices never
    /// see text the reply then loses), a reply that reaches the cap is cut back to a sentence
    /// end, "Weiter schreiben" counts its own part, and reasoning keeps the old wider cap.
    #[tokio::test]
    async fn the_answer_length_ends_a_reply_at_a_sentence_end() {
        let (url, got) = llama_words(&WORDS).await;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let cfg: crate::engine::EngineConfig = serde_json::from_str(r#"{ "llama_server": "x.exe", "model": "m.gguf" }"#).unwrap();
        let (_, profile) = store.import_engine_json(&cfg).unwrap().unwrap();
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)));
        let set_length = |n: i64, reasoning: &str| {
            let edit = store::ProfileEdit {
                sampler: Some(Sampler { n_predict: Some(n), ..Sampler::default() }),
                reasoning: Some(reasoning.into()),
                ..store::ProfileEdit::default()
            };
            hub.store(|s| s.update_profile(profile, &edit)).unwrap().unwrap();
        };
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        let sent = |path: &str| got.lock().unwrap().iter().rev().find(|(p, _)| p == path).unwrap().1.clone();

        // 10 tokens, from 8 on a sentence end counts: the second sentence is still running at
        // token 8 and ends with token 9, which token 10 confirms. The cap is the length.
        set_length(10, "off");
        hub.send(chat, "Und jetzt?".into()).unwrap();
        let (done, seen) = written(&mut rx).await;
        assert_eq!((done["content"].as_str(), done["error"].clone()), (Some("Sie geht los. Dann bleibt sie stehen."), Value::Null));
        assert_eq!(seen, " Sie geht los. Dann bleibt sie stehen.", "the devices saw exactly the reply");
        assert_eq!(done["timings"]["predicted"], 10, "the timings of the last token read");
        assert_eq!(sent("completion")["n_predict"], 10, "the hard cap is the length itself");
        let msgs = sent("apply-template")["messages"].as_array().unwrap().clone();
        let note = prompt::length_note(Language::De, 10).unwrap();
        assert!(msgs.last().unwrap()["content"].as_str().unwrap().ends_with(&format!("Und jetzt?\n\n[Anweisung für diese Antwort: {note}]")));
        assert!(!msgs[0]["content"].as_str().unwrap().contains(&note), "never in the head");
        let reply = hub.store(|s| s.messages(chat)).unwrap()[2].id;

        // "Weiter schreiben": the length counts the part written now, so it runs to its own
        // sentence end after 8 tokens (counting the whole reply it would end after " los.").
        hub.continue_reply(reply).unwrap();
        let (done, _) = written(&mut rx).await;
        assert_eq!(done["content"].as_str(), Some("Sie geht los. Dann bleibt sie stehen. Sie geht los. Dann bleibt sie stehen."));
        assert_eq!(sent("completion")["n_predict"], 10);
        assert!(sent("apply-template")["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().ends_with(&format!("{note}]")));

        // 8 tokens: the model server stops at the cap (8) before a sentence ended since token 6.
        // The running sentence is dropped, what was handed on before the mark stays.
        set_length(8, "off");
        hub.send(chat, "Weiter.".into()).unwrap();
        let (done, seen) = written(&mut rx).await;
        assert_eq!(sent("completion")["n_predict"], 8);
        assert_eq!((done["content"].as_str(), seen.as_str()), (Some("Sie geht los. Dann"), " Sie geht los. Dann"));

        // Reasoning on: the thinking shares the tokens, the old wider cap (5 + 2) and no soft stop.
        set_length(5, "on");
        hub.send(chat, "Noch einmal.".into()).unwrap();
        let (done, seen) = written(&mut rx).await;
        assert_eq!(sent("completion")["n_predict"], 7);
        assert_eq!((done["content"].as_str(), seen.as_str()), (Some("Sie geht los. Dann bleibt sie"), " Sie geht los. Dann bleibt sie"));
        assert_eq!(done["timings"]["predicted"], 7);

        // Shorter than 85 % of the length: the model's own end, the held-back last mark included.
        set_length(100, "off");
        hub.send(chat, "Und?".into()).unwrap();
        let (done, seen) = written(&mut rx).await;
        assert_eq!(done["content"].as_str(), Some("Sie geht los. Dann bleibt sie stehen. Und sieht sich um."));
        assert!(seen.ends_with("um."));
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A sentence end inside speech does not end a reply, and a reply that reaches the cap inside
    /// speech is cut back to the last sentence end and the quote is closed.
    #[tokio::test]
    async fn a_reply_never_ends_inside_speech() {
        let (url, got) = llama_words(&SPEECH).await;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let cfg: crate::engine::EngineConfig = serde_json::from_str(r#"{ "llama_server": "x.exe", "model": "m.gguf" }"#).unwrap();
        let (_, profile) = store.import_engine_json(&cfg).unwrap().unwrap();
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        let n_predict = || got.lock().unwrap().iter().rev().find(|(p, _)| p == "completion").unwrap().1["n_predict"].clone();
        let mut say = async |n: i64, text: &str| {
            let edit = store::ProfileEdit { sampler: Some(Sampler { n_predict: Some(n), ..Sampler::default() }), ..store::ProfileEdit::default() };
            hub.store(|s| s.update_profile(profile, &edit)).unwrap().unwrap();
            hub.send(chat, text.into()).unwrap();
            written(&mut rx).await
        };

        // 13 tokens, from 11 on: "weg." (token 7) ends inside the speech and lies before that;
        // the speech closes with token 12 and token 13 confirms its end.
        let (done, seen) = say(13, "Eins").await;
        assert_eq!(done["content"].as_str(), Some("Sie sagt: „Geh weg. Sofort sonst gehe ich.“"));
        assert_eq!(seen, " Sie sagt: „Geh weg. Sofort sonst gehe ich.“");
        assert_eq!(n_predict(), 13);

        // 9 tokens, from 7 on: "weg." ends a sentence inside the speech; the cap (9) is hit
        // while the next sentence runs. The reply goes back to "weg." and closes the quote; the
        // devices never saw " Sofort sonst".
        let (done, seen) = say(9, "Zwei").await;
        assert_eq!(done["content"].as_str(), Some("Sie sagt: „Geh weg.“"));
        assert_eq!(seen, " Sie sagt: „Geh weg.“");
        assert_eq!(n_predict(), 9);

        // Under 85 %: as written, speech open or not.
        let (done, _) = say(100, "Drei").await;
        assert_eq!(done["content"].as_str(), Some("Sie sagt: „Geh weg. Sofort sonst gehe ich.“ Dann geht sie."));
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Like `llama_receiver`, but every completion answers another text (" Antwort 1.", …).
    async fn llama_numbered() -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
        use axum::routing::post;
        let got: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let (a, b) = (got.clone(), got.clone());
        let app = axum::Router::new()
            .route(
                "/apply-template",
                post(move |axum::Json(v): axum::Json<Value>| async move {
                    let prompt = render(&v, true);
                    a.lock().unwrap().push(("apply-template".into(), v));
                    axum::Json(json!({ "prompt": prompt }))
                }),
            )
            .route(
                "/completion",
                post(move |axum::Json(v): axum::Json<Value>| async move {
                    let mut all = b.lock().unwrap();
                    all.push(("completion".into(), v));
                    let n = all.iter().filter(|(path, _)| path == "completion").count();
                    format!(
                        "data: {{\"content\":\" Antwort {n}.\",\"stop\":false}}\n\ndata: {{\"content\":\"\",\"stop\":true,\"timings\":{{\"prompt_n\":2,\"cache_n\":40,\"predicted_n\":3,\"predicted_per_second\":12.5}}}}\n\n"
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, got)
    }

    /// The events of one generation up to the snapshot that follows its `done`: (first `history`,
    /// `done`, last `history`).
    async fn written_again(rx: &mut broadcast::Receiver<String>) -> (Value, Value, Value) {
        let (mut first, mut done) = (None, None);
        loop {
            let ev: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            match ev["type"].as_str() {
                Some("history") if first.is_none() => first = Some(ev),
                Some("history") if done.is_some() => return (first.unwrap(), done.unwrap(), ev),
                Some("done") => done = Some(ev),
                _ => {}
            }
        }
    }

    /// "Neu generieren" writes another variant of the last reply from the very prompt the reply
    /// was written from; the earlier variant stays and can be chosen again. "Weiter schreiben"
    /// sends that prompt plus the reply's text and adds what comes to the active variant. Every
    /// device gets the chat's snapshot at the start (the reply as being written) and at the end.
    #[tokio::test]
    async fn the_last_reply_is_written_again_and_written_on() {
        let (url, got) = llama_numbered().await;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi.","post_history_instructions":"Bleib knapp."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        hub.send(chat, "Wohin?".into()).unwrap();
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        let m = hub.store(|s| s.messages(chat)).unwrap();
        let (greeting, user_msg, reply) = (m[0].id, m[1].id, m[2].id);
        assert_eq!((m[2].content.as_str(), m[2].swipes.clone()), ("Antwort 1.", None));
        let sent = |path: &str| got.lock().unwrap().iter().rev().find(|(p, _)| p == path).unwrap().1.clone();
        let first_template = sent("apply-template");
        assert!(first_template["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().contains("Bleib knapp."), "the tail of layout B");

        // Only the chat's last message, and only a reply to the user.
        assert_eq!(hub.regenerate(greeting), Err("not_last"));
        assert_eq!(hub.continue_reply(user_msg), Err("not_last"));
        assert_eq!(hub.regenerate(999), Err("message_missing"));
        assert!(rx.try_recv().is_err(), "a refusal sends nothing");

        hub.regenerate(reply).unwrap();
        let (start, done, end) = written_again(&mut rx).await;
        assert_eq!((start["generating"].as_i64(), start["busy"].as_bool(), start["messages"][2]["content"].as_str()), (Some(reply), Some(true), Some("")));
        assert_eq!((done["mode"].as_str(), done["id"].as_i64(), done["content"].as_str()), (Some("variant"), Some(reply), Some("Antwort 2.")));
        assert_eq!((done["removed"].as_bool(), done["kept"].as_bool(), done["error"].clone()), (Some(false), Some(true), Value::Null));
        let last = &end["messages"][2];
        assert_eq!((end["generating"].clone(), end["busy"].as_bool(), end["messages"].as_array().map(Vec::len)), (Value::Null, Some(false), Some(3)));
        assert_eq!((last["content"].as_str(), last["swipe_id"].as_u64(), last["swipes"].clone()), (Some("Antwort 2."), Some(1), json!(["Antwort 1.", "Antwort 2."])));
        assert_eq!(last["timings"]["cached"], 40);
        assert_eq!(sent("apply-template"), first_template, "the same head, history and tail as for the first reply");
        assert_eq!(sent("completion")["prompt"], render(&first_template, true));

        // The first variant again, and the model goes on from its last word.
        hub.set_swipe(reply, 0).unwrap();
        while !rx.recv().await.unwrap().contains("\"history\"") {}
        hub.continue_reply(reply).unwrap();
        let (start, done, end) = written_again(&mut rx).await;
        assert_eq!((start["generating"].as_i64(), start["messages"][2]["content"].as_str()), (Some(reply), Some("Antwort 1.")));
        assert_eq!((done["mode"].as_str(), done["content"].as_str(), done["kept"].as_bool()), (Some("continue"), Some("Antwort 1. Antwort 3."), Some(true)));
        let last = &end["messages"][2];
        assert_eq!((last["content"].as_str(), last["swipe_id"].as_u64()), (Some("Antwort 1. Antwort 3."), Some(0)));
        assert_eq!(last["swipes"], json!(["Antwort 1. Antwort 3.", "Antwort 2."]), "only the active variant grew");
        assert_eq!(sent("apply-template"), first_template);
        assert_eq!(sent("completion")["prompt"], format!("{}Antwort 1.", render(&first_template, true)), "the reply stays the character's open turn");

        // The story after a restart; a chat that went on no longer offers its old reply.
        hub.send(chat, "Und dann?".into()).unwrap();
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        assert_eq!(hub.regenerate(reply), Err("not_last"));
        drop(hub);
        let reopened = Store::open(&dir).unwrap();
        let m = reopened.messages(chat).unwrap();
        assert_eq!((m.len(), m[2].content.as_str(), m[2].swipe_id), (5, "Antwort 1. Antwort 3.", Some(0)));
        drop(reopened);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// While the model writes, no reply is written again or on (in any chat), and a reply being
    /// written again cannot be changed. Without an answer of the model the reply stays as it was.
    #[tokio::test]
    async fn writing_again_is_single_flight_and_leaves_the_reply_without_an_answer() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir)));
        let pair = |hub: &Hub| {
            let chat = hub.open_chat(id, true).unwrap().unwrap();
            let reply = hub.store(|s| {
                s.add_message(chat, "user", "Wohin?").unwrap();
                s.add_message(chat, "assistant", "Nach Amrum.").unwrap().id
            });
            (chat, reply)
        };
        let (chat, reply) = pair(&hub);
        let (other, other_reply) = pair(&hub);
        let greeting_only = hub.open_chat(id, true).unwrap().unwrap();
        let greeting = hub.store(|s| s.messages(greeting_only)).unwrap()[0].id;
        assert_eq!(hub.regenerate(greeting), Err("not_reply"), "a greeting answers no message");
        assert_eq!(hub.continue_reply(greeting), Err("not_reply"));
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();

        hub.send(other, "Und dann?".into()).unwrap(); // the reply task has not run yet
        assert_eq!(hub.regenerate(reply), Err("busy"), "the model writes in another chat");
        assert_eq!(hub.continue_reply(reply), Err("busy"));
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        assert_eq!(hub.regenerate(other_reply), Err("not_last"), "that chat went on");
        let last = hub.store(|s| s.messages(other)).unwrap().pop().unwrap();
        assert_eq!(hub.regenerate(last.id), Err("not_reply"), "the user's own message");

        hub.regenerate(reply).unwrap(); // the task has not run yet
        assert_eq!(hub.regenerate(reply), Err("busy"));
        assert_eq!(hub.continue_reply(reply), Err("busy"));
        assert_eq!(hub.send(chat, "x".into()), Err("busy"));
        assert_eq!(hub.edit_message(reply, "x"), Err("busy"));
        assert_eq!(hub.rewind(reply), Err("busy"));
        let (start, done, end) = written_again(&mut rx).await;
        assert_eq!(start["generating"].as_i64(), Some(reply));
        assert_eq!((done["mode"].as_str(), done["removed"].as_bool(), done["kept"].as_bool()), (Some("variant"), Some(false), Some(false)));
        assert!(done["error"].as_str().unwrap().contains("apply-template failed"));
        assert_eq!((end["messages"][2]["content"].as_str(), end["messages"][2].get("swipes"), end["generating"].clone()), (Some("Nach Amrum."), None, Value::Null));
        let m = hub.store(|s| s.messages(chat)).unwrap();
        assert_eq!((m.len(), m[2].content.as_str(), m[2].swipes.clone()), (3, "Nach Amrum.", None));

        hub.continue_reply(reply).unwrap();
        let (start, done, _) = written_again(&mut rx).await;
        assert_eq!(start["messages"][2]["content"], "Nach Amrum.");
        assert_eq!((done["mode"].as_str(), done["kept"].as_bool()), (Some("continue"), Some(false)));
        assert_eq!(hub.store(|s| s.messages(chat)).unwrap()[2].content, "Nach Amrum.");
        assert!(hub.edit_message(reply, "Nach Föhr.").is_ok(), "free once the writing ended");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A message longer than the limit is neither sent nor stored by an edit; one at the limit is.
    #[tokio::test]
    async fn overlong_messages_are_refused() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let greeting = hub.store(|s| s.messages(chat)).unwrap()[0].id;
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        let too_long = "ä".repeat(MESSAGE_MAX_CHARS + 1);
        let at_limit = format!("  {}  ", "ä".repeat(MESSAGE_MAX_CHARS));

        assert_eq!(hub.send(chat, too_long.clone()), Err("too_long"));
        assert_eq!(hub.edit_message(greeting, &too_long), Err("too_long"));
        assert!(rx.try_recv().is_err(), "a refused message sends nothing");
        assert_eq!(hub.store(|s| s.messages(chat)).unwrap().len(), 1, "nothing stored");

        hub.edit_message(greeting, &at_limit).unwrap();
        hub.send(chat, at_limit.clone()).unwrap();
        while !rx.recv().await.unwrap().contains("\"done\"") {}
        let m = hub.store(|s| s.messages(chat)).unwrap();
        assert_eq!((m[0].content.chars().count(), m[1].content.chars().count()), (MESSAGE_MAX_CHARS, MESSAGE_MAX_CHARS));
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A panic while one of the hub's locks is held does not take every later request down.
    #[tokio::test]
    async fn a_panic_under_a_lock_does_not_stop_the_hub() {
        let (url, _) = llama_receiver().await;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hub.store(|_| panic!("a bug in a route"))));
        assert!(panicked.is_err());
        let h = hub.clone();
        assert!(std::thread::spawn(move || {
            let _idle = h.idle.lock().unwrap_or_else(PoisonError::into_inner);
            let _bench = h.bench.lock().unwrap_or_else(PoisonError::into_inner);
            let _llama = h.llama.lock().unwrap_or_else(PoisonError::into_inner);
            panic!("a bug while every other lock is held");
        })
        .join()
        .is_err());
        assert!(hub.inner.is_poisoned() && hub.idle.is_poisoned() && hub.bench.is_poisoned() && hub.llama.is_poisoned());

        assert!(!hub.generating() && !hub.unloaded() && hub.bench_run().is_none());
        assert!(hub.llama_url().starts_with("http://127.0.0.1:"));
        hub.send(chat, "Noch da?".into()).unwrap();
        let done: Value = loop {
            let ev: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            if ev["type"] == "done" {
                break ev;
            }
        };
        assert_eq!((done["content"].as_str(), done["error"].clone()), (Some("Hallo"), Value::Null));
        assert_eq!(hub.store(|s| s.messages(chat)).unwrap().len(), 3);
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A job outside any chat (the card builder) holds the model server like a reply does: no
    /// second job, no reply, no unload or benchmark meanwhile; dropping it frees everything.
    #[tokio::test]
    async fn side_job_takes_the_one_generation_slot() {
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hello."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let job = hub.side_begin().unwrap();
        assert!(hub.generating());
        assert_eq!(hub.side_begin().err(), Some("busy"), "one job at a time");
        assert_eq!(hub.send(chat, "Hi".into()), Err("busy"));
        assert_eq!(hub.unload(), Err("busy"));
        assert_eq!(hub.bench_begin(1), Err("busy"));
        drop(job);
        assert!(!hub.generating());
        assert!(hub.side_begin().is_ok());
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    // ------------------------------------------------------------ context, greetings

    /// Tokens of a text as the model server below counts them: one per started 4 bytes.
    fn tokens_of(text: &str) -> usize {
        text.len().div_ceil(4)
    }

    /// A real local HTTP server in llama-server's place with a context of `n_ctx` tokens: `/props`
    /// says it (None: no `/props`, the context is unknown), `/tokenize` counts with `tokens_of`,
    /// `/apply-template` renders ChatML, and `/completion` refuses a prompt over `refuse_over`
    /// tokens the way llama-server does (400 `exceed_context_size_error`). Keeps every prompt.
    async fn llama_context(n_ctx: Option<u64>, refuse_over: usize) -> (String, Arc<Mutex<Vec<String>>>) {
        use axum::routing::{get, post};
        let prompts: Arc<Mutex<Vec<String>>> = Arc::default();
        let kept = prompts.clone();
        let mut app = axum::Router::new()
            .route("/apply-template", post(|axum::Json(v): axum::Json<Value>| async move { axum::Json(json!({ "prompt": render(&v, true) })) }))
            .route(
                "/tokenize",
                post(|axum::Json(v): axum::Json<Value>| async move { axum::Json(json!({ "tokens": vec![1; tokens_of(v["content"].as_str().unwrap())] })) }),
            )
            .route(
                "/completion",
                post(move |axum::Json(v): axum::Json<Value>| async move {
                    let prompt = v["prompt"].as_str().unwrap().to_string();
                    let n = tokens_of(&prompt);
                    kept.lock().unwrap().push(prompt);
                    if n > refuse_over {
                        let body = json!({ "error": { "code": 400, "message": "the request exceeds the available context size, try increasing it", "type": "exceed_context_size_error", "n_prompt_tokens": n, "n_ctx": refuse_over } });
                        return (axum::http::StatusCode::BAD_REQUEST, body.to_string());
                    }
                    (axum::http::StatusCode::OK, "data: {\"content\":\"Hallo\",\"stop\":false}\n\ndata: {\"content\":\"\",\"stop\":true}\n\n".to_string())
                }),
            );
        if let Some(n) = n_ctx {
            app = app.route("/props", get(move || async move { axum::Json(json!({ "model_path": "C:\\m\\small.gguf", "default_generation_settings": { "n_ctx": n } })) }));
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, prompts)
    }

    /// Sends `text` and waits for its `done` event.
    async fn say(hub: &Arc<Hub>, rx: &mut broadcast::Receiver<String>, chat: i64, text: &str) -> Value {
        hub.send(chat, text.into()).unwrap();
        loop {
            let ev: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
            if ev["type"] == "done" {
                return ev;
            }
        }
    }

    /// The turns of a ChatML prompt: (role, text).
    fn chatml_turns(prompt: &str) -> Vec<(String, String)> {
        prompt
            .split("<|im_start|>")
            .filter(|t| !t.is_empty())
            .map(|t| {
                let (role, text) = t.split_once('\n').unwrap_or((t, ""));
                (role.to_string(), text.trim_end_matches('\n').trim_end_matches("<|im_end|>").to_string())
            })
            .collect()
    }

    /// A chat far longer than the model's context still gets its answer: the oldest turns after
    /// the greeting are left out in blocks, the prompt fits, keeps the head with the fixed rule,
    /// the greeting, the newest message and strictly alternating turns, and from one reply to the
    /// next it only grows at its end until the next block goes. The stored chat keeps everything.
    #[tokio::test]
    async fn a_chat_longer_than_the_context_is_cut_from_its_oldest_end() {
        let n_ctx = 3000;
        let (url, prompts) = llama_context(Some(n_ctx), n_ctx as usize).await;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Willkommen im Leuchtturm.","post_history_instructions":"Bleib knapp."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let line = |i: usize| format!("Runde {i}: {}", "Wellen schlagen an die Mauer. ".repeat(6));
        hub.store(|s| {
            for i in 0..60 {
                s.add_message(chat, "user", &format!("Frage {}", line(i))).unwrap();
                s.add_message(chat, "assistant", &format!("Antwort {}", line(i))).unwrap();
            }
        });
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        let budget = prompt_budget(n_ctx, None, false);
        let mut seen: Vec<Vec<(String, String)>> = vec![];
        for round in 0..12 {
            let done = say(&hub, &mut rx, chat, &format!("Neue Frage {round}: {}", line(100 + round))).await;
            assert_eq!((done["content"].as_str(), done["error"].clone()), (Some("Hallo"), Value::Null), "round {round}");
            let prompt = prompts.lock().unwrap().last().unwrap().clone();
            assert!(tokens_of(&prompt) <= budget, "round {round}: {} tokens over {budget}", tokens_of(&prompt));
            let turns = chatml_turns(&prompt);
            assert_eq!(turns[0].0, "system");
            assert!(turns[0].1.contains("# Figur: Mara"), "the system message includes the character card");
            let roles: Vec<&str> = turns[1..].iter().map(|(r, _)| r.as_str()).collect();
            assert_eq!(roles.last(), Some(&"assistant"), "the open turn of the reply");
            for (i, r) in roles[..roles.len() - 1].iter().enumerate() {
                assert_eq!(*r, if i % 2 == 0 { "user" } else { "assistant" }, "round {round}: strict alternation {roles:?}");
            }
            assert_eq!(turns[1].1, "(Die Szene beginnt.)");
            assert!(turns[2].1.starts_with("Willkommen im Leuchtturm."), "the greeting stays: {}", turns[2].1);
            assert!(!prompt.contains("Frage Runde 0:"), "the oldest rounds are out");
            let newest = &turns[turns.len() - 2].1;
            let tail = format!("[Anweisung für diese Antwort: Bleib knapp.\n{}]", prompt::length_note(Language::De, DEFAULT_N_PREDICT).unwrap());
            assert!(newest.starts_with(&format!("Neue Frage {round}:")) && newest.ends_with(&tail), "{newest}");
            seen.push(turns);
        }
        // Between two cuts every prompt starts with the one before (without its last user turn,
        // which carried the tail, and the open reply turn): the model server's cache holds.
        let mut grown = 0;
        for w in seen.windows(2) {
            let before = &w[0][..w[0].len() - 2];
            if w[1].starts_with(before) {
                grown += 1;
            }
        }
        assert!(grown >= 8, "most replies only add at the end ({grown} of 11)");
        assert!(grown < 11, "a block went at least once");
        assert_eq!(hub.store(|s| s.messages(chat)).unwrap().len(), 1 + 120 + 24, "the stored chat keeps everything");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Head and newest message alone longer than the context: no request reaches the model, the
    /// `done` event names `context_full`, the user's message stays and the empty reply goes.
    #[tokio::test]
    async fn context_full_when_the_head_alone_does_not_fit() {
        let (url, prompts) = llama_context(Some(1024), 1024).await;
        let (mut store, dir) = temp_store();
        let long = "Der Leuchtturm steht auf einer Klippe über dem Meer. ".repeat(120);
        let card = json!({ "name": "Mara", "first_mes": "Hallo.", "description": long }).to_string();
        let id = store.add_character(&sample(&card), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        let done = say(&hub, &mut rx, chat, "Hallo Mara").await;
        assert_eq!((done["error"].as_str(), done["removed"].as_bool()), (Some(CONTEXT_FULL), Some(true)));
        assert!(prompts.lock().unwrap().is_empty(), "nothing went to the model");
        let m = hub.store(|s| s.messages(chat)).unwrap();
        assert_eq!(m.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["Hallo.", "Hallo Mara"]);
        assert!(hub.send(chat, "Noch da?".into()).is_ok(), "the hub is free again");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A model server that does not say its context: the prompt goes as it is, and when the
    /// model server refuses it as too long the `done` event names `context_full` too, not the raw
    /// 400. The estimate for a model server that cannot count stays on the safe side, and the
    /// room for the reply is the hard cap (the profile's `n_predict`, wider with reasoning), at most half the context.
    #[tokio::test]
    async fn an_overflow_the_model_server_refuses_is_context_full() {
        let (url, prompts) = llama_context(None, 200).await;
        let (mut store, dir) = temp_store();
        let id = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hallo."}"#), None).unwrap().id;
        let hub = Arc::new(Hub::new(store, url, Logger::start(&dir)));
        let chat = hub.open_chat(id, false).unwrap().unwrap();
        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        let done = say(&hub, &mut rx, chat, "Hallo Mara").await;
        assert_eq!(done["error"].as_str(), Some(CONTEXT_FULL));
        assert_eq!(prompts.lock().unwrap().len(), 1, "the whole prompt went once");
        assert!(estimate_tokens("Привет, как дела?") >= tokens_of("Привет, как дела?"));
        assert_eq!(prompt_budget(4096, Some(150), false), 4096 - 150 - CONTEXT_MARGIN, "room for the hard cap: the length");
        assert_eq!(prompt_budget(4096, Some(150), true), 4096 - 225 - CONTEXT_MARGIN, "with reasoning the wider cap");
        assert_eq!(prompt_budget(2048, Some(800), true), 2048 - 1000 - CONTEXT_MARGIN);
        assert_eq!(prompt_budget(2048, Some(4000), false), 1024 - CONTEXT_MARGIN, "at most half the context for the reply");
        assert_eq!(prompt_budget(8192, None, false), 8192 - 800 - CONTEXT_MARGIN);
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The card's alternate greetings are variants of a new chat's first message: `first_mes` is
    /// variant 0 and active, swiping shows another greeting (for every device, and in the
    /// prompt), and a card without alternate ones opens without variants as before.
    #[test]
    fn a_new_chat_offers_every_greeting_as_a_variant() {
        let (mut store, dir) = temp_store();
        let card = r#"{"name":"Mara","first_mes":"Evening, {{user}}.","alternate_greetings":["Morning, {{user}}.","  ","<BOT> waves from the tower."]}"#;
        let id = store.add_character(&sample(card), None).unwrap().id;
        let plain = store.add_character(&sample(r#"{"name":"Theo","first_mes":"Moin."}"#), None).unwrap().id;
        store.set_user_name("Lukas").unwrap();
        let hub = Hub::new(store, "http://127.0.0.1:1".into(), Logger::start(&dir));
        let chat = hub.open_chat(id, true).unwrap().unwrap();
        let first = &hub.store(|s| s.messages(chat)).unwrap()[0];
        assert_eq!(first.content, "Evening, Lukas.");
        assert_eq!((first.swipe_id, first.swipes.clone()), (Some(0), Some(vec!["Evening, Lukas.".into(), "Morning, Lukas.".into(), "Mara waves from the tower.".into()])));

        let (_, mut rx) = hub.open_view(chat).unwrap().unwrap();
        hub.set_swipe(first.id, 2).unwrap();
        let history = loop {
            let ev: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
            if ev["type"] == "history" {
                break ev;
            }
        };
        assert_eq!((history["messages"][0]["content"].as_str(), history["messages"][0]["swipe_id"].as_u64()), (Some("Mara waves from the tower."), Some(2)));
        let (msgs, _) = hub.inner.lock().unwrap().prompt(chat, None).unwrap().unwrap();
        assert_eq!(msgs[2].content, "Mara waves from the tower.", "the prompt reads the chosen greeting");
        hub.set_swipe(first.id, 0).unwrap();
        assert_eq!(hub.store(|s| s.messages(chat)).unwrap()[0].content, "Evening, Lukas.");

        let other = hub.open_chat(plain, true).unwrap().unwrap();
        let m = hub.store(|s| s.messages(other)).unwrap();
        assert_eq!((m[0].content.as_str(), m[0].swipes.clone(), m[0].swipe_id), ("Moin.", None, None));
        let none = hub.store(|s| s.create_chat_with_greetings(plain, &[])).unwrap();
        assert!(hub.store(|s| s.messages(none)).unwrap().is_empty(), "no greeting, no message");
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }
}
