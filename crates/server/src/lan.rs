//! Phones in the home network ("Handy verbinden"): switched on and off while the app runs.
//!
//! On: a second listener on every IPv4 address (port kept in the setting `lan_port`, so a QR code
//! stays valid after a restart) serves the same app as the local one, but every request over it is
//! marked [`ViaLan`] and needs the phones' own token (setting `lan_token`; the desktop window's
//! token never leaves the PC). A 6-digit pairing code (10 minutes; 5 wrong ones make that address
//! wait, 20 from all addresses replace the code) hands that token to a phone without a camera
//! (`POST /api/pair`), and a UDP socket on port
//! [`DISCOVERY_PORT`] answers `LORELAMP_DISCOVER 1` so the phone app finds the PC.
//! Off: both sockets closed, every phone connection cut. A new token cuts them too.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::serve::{Listener, ListenerExt};
use axum::Router;
use rand::distr::{Alphanumeric, SampleString};
use rand::Rng;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::task::JoinHandle;

use crate::chat::Hub;
use crate::logs::{Entry, Level};

/// The phones' port when nothing is stored yet; taken ones are skipped up to the end of the range.
const PORTS: std::ops::RangeInclusive<u16> = 7860..=7879;
/// UDP port of the search in the Wi-Fi (contract with the phone app).
pub const DISCOVERY_PORT: u16 = 47823;
/// The one datagram the search answers.
pub const DISCOVER: &[u8] = b"LORELAMP_DISCOVER 1";
/// Where both sockets bind: every IPv4 address; in the crate's tests loopback only (and any free
/// ports), so a test never opens the PC to the network.
#[cfg(not(test))]
const BIND: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
#[cfg(test)]
const BIND: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
#[cfg(not(test))]
const SEARCH_PORT: u16 = DISCOVERY_PORT;
#[cfg(test)]
const SEARCH_PORT: u16 = 0;
const CODE_TTL: Duration = Duration::from_secs(10 * 60);
/// Wrong codes in a row from one address, then that address waits ([`IP_LOCK`], doubled at every
/// further lock up to [`IP_LOCK_MAX`]); other addresses go on. So one device in the Wi-Fi cannot keep
/// the right phone from pairing.
const IP_TRIES: u8 = 5;
const IP_LOCK: Duration = Duration::from_secs(30);
const IP_LOCK_MAX: Duration = Duration::from_secs(15 * 60);
/// Wrong codes on one code from all addresses together (many addresses taking turns): then the code
/// is replaced and pairing pauses for [`ALL_LOCK`]. The PC shows the new code at once, so the right
/// phone waits at most that long.
const CODE_WRONG_MAX: u16 = 20;
const ALL_LOCK: Duration = Duration::from_secs(30);
/// Addresses are forgotten an hour after their last wrong code (and while waiting never); at most
/// [`IP_KEEP`] are kept (the longest quiet goes first).
const IP_FORGET: Duration = Duration::from_secs(60 * 60);
const IP_KEEP: usize = 1024;

/// Marks a request that came in over the LAN listener (set as a request extension).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ViaLan;

/// What a pairing attempt gets.
#[derive(Debug, PartialEq)]
pub(crate) enum Paired {
    Token(Arc<str>),
    Wrong,
    TooMany,
    Off,
}

pub(crate) struct Lan {
    hub: Arc<Hub>,
    /// The local listener's port; the phones' one is always another.
    main_port: u16,
    token: RwLock<Arc<str>>,
    run: Mutex<Run>,
    /// One switch at a time.
    switching: tokio::sync::Mutex<()>,
    conns: Arc<Mutex<Vec<Weak<std::net::TcpStream>>>>,
    pair: Mutex<Pair>,
    /// The app the LAN listener serves (the local app, with every request marked [`ViaLan`]).
    app: OnceLock<Router>,
    bind_ip: IpAddr,
    discovery_port: u16,
}

#[derive(Default)]
struct Run {
    on: bool,
    port: u16,
    udp_port: u16,
    http: Option<JoinHandle<()>>,
    udp: Option<JoinHandle<()>>,
    problem: Option<&'static str>,
}

#[derive(Default)]
struct Pair {
    code: Option<(String, Instant)>,
    /// Wrong codes on the current code, from all addresses.
    wrong: u16,
    /// Pairing paused for everyone (after [`CODE_WRONG_MAX`]).
    locked_until: Option<Instant>,
    senders: HashMap<IpAddr, Sender>,
}

/// The wrong codes of one address.
struct Sender {
    /// In a row, since its last lock.
    wrong: u8,
    /// How often it was locked (the wait doubles each time).
    locks: u32,
    until: Option<Instant>,
    last: Instant,
}

impl Sender {
    fn waiting(&self, now: Instant) -> bool {
        self.until.is_some_and(|u| now < u)
    }
}

/// Drops the addresses quiet for [`IP_FORGET`] (never one that waits), then the quietest ones above
/// [`IP_KEEP`].
fn forget_quiet(senders: &mut HashMap<IpAddr, Sender>, now: Instant) {
    senders.retain(|_, s| s.waiting(now) || now.duration_since(s.last) < IP_FORGET);
    while senders.len() >= IP_KEEP {
        let Some(quietest) = senders.iter().min_by_key(|(_, s)| s.last).map(|(ip, _)| *ip) else { break };
        senders.remove(&quietest);
    }
}

/// [`IP_LOCK`] doubled for every earlier lock, at most [`IP_LOCK_MAX`].
fn ip_lock(locks: u32) -> Duration {
    IP_LOCK.saturating_mul(1u32 << locks.saturating_sub(1).min(10)).min(IP_LOCK_MAX)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// An address a phone may use: private ranges, link-local and the shared range VPN meshes use
/// (100.64.0.0/10), plus loopback (a test on this PC).
pub fn reachable_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local() || shared_range(v4),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

fn shared_range(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    a == 100 && (64..128).contains(&b)
}

/// Addresses worth offering for the QR code: private and shared-range ones (no link-local: that
/// is a network without a router).
fn offered(ip: Ipv4Addr) -> bool {
    ip.is_private() || shared_range(ip)
}

/// This PC's name for the search answer (at most 63 characters).
fn pc_name() -> String {
    let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(63).collect();
    if name.is_empty() {
        "PC".into()
    } else {
        name
    }
}

fn new_token() -> String {
    Alphanumeric.sample_string(&mut rand::rng(), 32)
}

/// Six digits from the thread's CSPRNG (ChaCha).
fn new_code() -> String {
    format!("{:06}", rand::rng().random_range(0..1_000_000u32))
}

/// Same answer time for every wrong code of the same length.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Lan {
    /// Loads the phones' token (made and kept on first use).
    pub fn new(hub: Arc<Hub>, main_port: u16) -> Lan {
        let token = match hub.store(|s| s.lan_token()) {
            Ok(Some(t)) => t,
            _ => {
                let t = new_token();
                let _ = hub.store(|s| s.set_lan_token(&t));
                t
            }
        };
        // A test server binds loopback only (`RP_LAN_BIND=127.0.0.1`) and its own search port.
        let bind_ip = std::env::var("RP_LAN_BIND").ok().and_then(|v| v.parse().ok()).unwrap_or(BIND);
        let discovery_port = std::env::var("RP_DISCOVERY_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(SEARCH_PORT);
        Lan {
            hub,
            main_port,
            token: RwLock::new(token.into()),
            run: Mutex::default(),
            switching: tokio::sync::Mutex::new(()),
            conns: Arc::default(),
            pair: Mutex::default(),
            app: OnceLock::new(),
            bind_ip,
            discovery_port,
        }
    }

    pub fn set_app(&self, app: Router) {
        let _ = self.app.set(app);
    }

    /// The switch as stored (the next start follows it).
    pub fn stored_enabled(&self) -> bool {
        self.hub.store(|s| s.lan_enabled()).unwrap_or(false)
    }

    pub fn is_on(&self) -> bool {
        lock(&self.run).on
    }

    pub fn token(&self) -> Arc<str> {
        self.token.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The phones' port while on (else 0).
    pub fn port(&self) -> u16 {
        let r = lock(&self.run);
        if r.on {
            r.port
        } else {
            0
        }
    }

    /// The search socket's port while on (else 0).
    #[cfg(test)]
    pub fn discovery_bound(&self) -> u16 {
        lock(&self.run).udp_port
    }

    fn log(&self, level: Level, what: &str, data: Value) {
        self.hub.log().write(Entry::new(level, "server", "lan", what).data(data));
    }

    /// Opens the phones' listener and the search socket. Err `port_busy` when no port of the range
    /// is free (nothing changes then).
    pub async fn enable(&self) -> Result<(), &'static str> {
        let _one = self.switching.lock().await;
        if self.is_on() {
            return Ok(());
        }
        let Some(app) = self.app.get().cloned() else { return Err("not_ready") };
        let ip = self.bind_ip;
        let stored = self.hub.store(|s| s.lan_port()).ok().flatten();
        let mut listener = None;
        for port in stored.into_iter().chain(cfg!(test).then_some(0)).chain(PORTS) {
            if port == self.main_port && port != 0 {
                continue;
            }
            if let Ok(l) = TcpListener::bind(SocketAddr::new(ip, port)).await {
                listener = Some(l);
                break;
            }
        }
        let Some(listener) = listener else {
            lock(&self.run).problem = Some("port_busy");
            self.log(Level::Warn, "phones: no free port", json!({ "from": PORTS.start(), "to": PORTS.end() }));
            return Err("port_busy");
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
        if stored != Some(port) {
            let _ = self.hub.store(|s| s.set_lan_port(port));
        }
        let lan_listener = LanListener { inner: listener, conns: self.conns.clone() }.tap_io(|_| {});
        let service = app.into_make_service_with_connect_info::<SocketAddr>();
        let http = tokio::spawn(async move {
            let _ = axum::serve(lan_listener, service).await;
        });
        // The search is a help only: without it the phone types the address.
        let discovery_port = self.discovery_port;
        let (udp, udp_port) = match UdpSocket::bind(SocketAddr::new(ip, discovery_port)).await {
            Ok(sock) => {
                let bound = sock.local_addr().map(|a| a.port()).unwrap_or_default();
                (Some(tokio::spawn(answer_search(sock, port))), bound)
            }
            Err(e) => {
                self.log(Level::Warn, "phones: search port not available", json!({ "port": discovery_port, "error": e.to_string() }));
                (None, 0)
            }
        };
        {
            let mut r = lock(&self.run);
            *r = Run { on: true, port, udp_port, http: Some(http), udp, problem: None };
        }
        *lock(&self.pair) = Pair::default();
        self.log(Level::Info, "phones may connect", json!({ "port": port, "search": udp_port != 0 }));
        Ok(())
    }

    /// Closes both sockets and cuts every phone connection.
    pub async fn disable(&self) {
        let _one = self.switching.lock().await;
        let (http, udp, was_on) = {
            let mut r = lock(&self.run);
            let was_on = r.on;
            r.on = false;
            r.udp_port = 0;
            (r.http.take(), r.udp.take(), was_on)
        };
        // Waiting for the aborted tasks: their sockets are closed when this returns.
        for task in [http, udp].into_iter().flatten() {
            task.abort();
            let _ = task.await;
        }
        self.cut_all();
        *lock(&self.pair) = Pair::default();
        if was_on {
            self.log(Level::Info, "phones disconnected (switched off)", json!({}));
        }
    }

    /// Stops everything at once (the server stops).
    pub fn abort(&self) {
        let mut r = lock(&self.run);
        r.on = false;
        for task in [r.http.take(), r.udp.take()].into_iter().flatten() {
            task.abort();
        }
        drop(r);
        self.cut_all();
    }

    /// "Alle Handys trennen": a new token (old QR codes and paired phones are refused), a new
    /// code, every phone connection cut. The desktop window keeps its own token.
    pub fn new_token(&self) {
        let t = new_token();
        let _ = self.hub.store(|s| s.set_lan_token(&t));
        *self.token.write().unwrap_or_else(|p| p.into_inner()) = t.into();
        self.cut_all();
        lock(&self.pair).code = None;
        self.log(Level::Info, "phones disconnected (new token)", json!({}));
    }

    fn cut_all(&self) {
        let conns: Vec<_> = lock(&self.conns).drain(..).collect();
        for conn in conns.iter().filter_map(Weak::upgrade) {
            let _ = conn.shutdown(Shutdown::Both);
        }
    }

    /// The current code and its seconds left; a new one when there is none or it ran out. None
    /// while off.
    pub fn code(&self) -> Option<(String, u64)> {
        if !self.is_on() {
            return None;
        }
        let mut p = lock(&self.pair);
        let now = Instant::now();
        let fresh = p.code.as_ref().is_some_and(|(_, made)| now.duration_since(*made) < CODE_TTL);
        if !fresh {
            // A fresh budget of wrong codes per code.
            p.code = Some((new_code(), now));
            p.wrong = 0;
        }
        let (code, made) = p.code.clone()?;
        Some((code, CODE_TTL.saturating_sub(now.duration_since(made)).as_secs()))
    }

    /// "Neuer Code".
    pub fn renew_code(&self) {
        let mut p = lock(&self.pair);
        p.code = None;
        p.wrong = 0;
    }

    /// Seconds until codes are taken again from every address after too many wrong ones (0: not
    /// paused; one address waiting alone does not count).
    pub fn locked_for(&self) -> u64 {
        lock(&self.pair).locked_until.map_or(0, |until| until.saturating_duration_since(Instant::now()).as_secs())
    }

    #[cfg(test)]
    pub fn expire_code(&self) {
        if let Some((_, made)) = lock(&self.pair).code.as_mut() {
            *made = made.checked_sub(CODE_TTL).expect("PC runs longer than 10 minutes");
        }
    }

    /// Lifts every wait (the pause for all and each address's).
    #[cfg(test)]
    pub fn unlock(&self) {
        let mut p = lock(&self.pair);
        p.locked_until = None;
        p.senders.clear();
    }

    /// A phone at `from` sends the code it was shown. Right: the token, and the code is used up.
    /// Wrong: counted for `from` (the fifth in a row makes it wait, see [`IP_TRIES`]) and for the code
    /// (the twentieth from all addresses replaces it and pauses pairing, see [`CODE_WRONG_MAX`]).
    pub fn pair(&self, code: &str, from: IpAddr) -> Paired {
        if !self.is_on() {
            return Paired::Off;
        }
        let mut p = lock(&self.pair);
        let now = Instant::now();
        if p.locked_until.is_some_and(|until| now < until) || p.senders.get(&from).is_some_and(|s| s.waiting(now)) {
            return Paired::TooMany;
        }
        p.locked_until = None;
        let valid = p.code.as_ref().filter(|(_, made)| now.duration_since(*made) < CODE_TTL);
        if valid.is_some_and(|(c, _)| same(c, code.trim())) {
            // Used up; the addresses that sent wrong ones keep waiting.
            p.code = None;
            p.wrong = 0;
            p.senders.remove(&from);
            drop(p);
            self.log(Level::Info, "phone paired with the code", json!({}));
            return Paired::Token(self.token());
        }
        forget_quiet(&mut p.senders, now);
        let s = p.senders.entry(from).or_insert(Sender { wrong: 0, locks: 0, until: None, last: now });
        s.wrong += 1;
        s.last = now;
        let sender_locked = s.wrong >= IP_TRIES;
        if sender_locked {
            s.wrong = 0;
            s.locks = s.locks.saturating_add(1);
            s.until = Some(now + ip_lock(s.locks));
        }
        let wait = s.until.map(|u| u.saturating_duration_since(now).as_secs());
        p.wrong += 1;
        if p.wrong >= CODE_WRONG_MAX {
            p.code = None;
            p.wrong = 0;
            p.locked_until = Some(now + ALL_LOCK);
            drop(p);
            self.log(Level::Warn, "pairing paused for 30 s after 20 wrong codes; new code", json!({ "wrong": CODE_WRONG_MAX }));
            return Paired::TooMany;
        }
        drop(p);
        if sender_locked {
            // The address is not logged (a phone's address in the home network is personal enough).
            self.log(Level::Warn, "pairing: one address waits after 5 wrong codes", json!({ "wait_s": wait }));
            return Paired::TooMany;
        }
        Paired::Wrong
    }

    /// The addresses to offer ({ip, adapter}), the default first, and the chosen one.
    fn addresses(&self) -> (Vec<hw::NetAddress>, Option<Ipv4Addr>) {
        let mut list: Vec<hw::NetAddress> = hw::ipv4_addresses().into_iter().filter(|a| offered(a.ip)).collect();
        let route = match crate::lan_ip() {
            Some(IpAddr::V4(v4)) if offered(v4) => Some(v4),
            _ => None,
        };
        if let Some(r) = route {
            match list.iter().position(|a| a.ip == r) {
                Some(i) => {
                    let a = list.remove(i);
                    list.insert(0, a);
                }
                None => list.insert(0, hw::NetAddress { ip: r, adapter: String::new() }),
            }
        }
        list.dedup_by(|a, b| a.ip == b.ip);
        let picked = self.hub.store(|s| s.lan_ip()).ok().flatten().and_then(|s| s.parse::<Ipv4Addr>().ok());
        let chosen = picked.filter(|p| list.iter().any(|a| a.ip == *p)).or_else(|| list.first().map(|a| a.ip));
        (list, chosen)
    }

    /// Picks the address for the QR code; false when the PC has no such address.
    pub fn choose(&self, ip: &str) -> bool {
        let Ok(ip) = ip.parse::<Ipv4Addr>() else { return false };
        if !self.addresses().0.iter().any(|a| a.ip == ip) {
            return false;
        }
        self.hub.store(|s| s.set_lan_ip(&ip.to_string())).is_ok()
    }

    /// The address for phones (the QR code's content), while on.
    pub fn url(&self) -> Option<String> {
        let port = self.port();
        if port == 0 {
            return None;
        }
        let ip = self.addresses().1?;
        Some(format!("http://{ip}:{port}/?token={}", self.token()))
    }

    /// `GET /lan`.
    pub fn status(&self) -> Value {
        let (list, chosen) = self.addresses();
        let (on, problem) = {
            let r = lock(&self.run);
            (r.on, r.problem)
        };
        let port = if on { self.port() } else { self.hub.store(|s| s.lan_port()).ok().flatten().unwrap_or(*PORTS.start()) };
        let code = self.code();
        let url = chosen.filter(|_| on).map(|ip| format!("http://{ip}:{port}/?token={}", self.token()));
        json!({
            "enabled": on,
            "port": port,
            "addresses": list.iter().map(|a| json!({ "ip": a.ip.to_string(), "adapter": a.adapter })).collect::<Vec<_>>(),
            "ip": chosen.map(|ip| ip.to_string()),
            "url": url,
            "code": code.as_ref().map(|(c, _)| c.clone()),
            "code_expires_in": code.map(|(_, left)| left),
            "locked_for": self.locked_for(),
            "problem": problem,
            "search": on && lock(&self.run).udp_port != 0,
        })
    }
}

/// Answers the phone app's search (`LORELAMP_DISCOVER 1`) from the home network with this PC's
/// name and the phones' port; anything else, or from outside the private ranges, gets nothing.
async fn answer_search(sock: UdpSocket, port: u16) {
    let reply = json!({ "app": "lorelamp", "v": 1, "name": pc_name(), "port": port }).to_string();
    let mut buf = [0u8; 64];
    loop {
        // Windows reports an ICMP "port unreachable" of an earlier answer as an error here: go on.
        let Ok((n, from)) = sock.recv_from(&mut buf).await else { continue };
        if &buf[..n] == DISCOVER && reachable_ip(from.ip()) {
            let _ = sock.send_to(reply.as_bytes(), from).await;
        }
    }
}

/// The LAN listener: every connection is kept (weakly) so switching off can cut it.
struct LanListener {
    inner: TcpListener,
    conns: Arc<Mutex<Vec<Weak<std::net::TcpStream>>>>,
}

impl Listener for LanListener {
    type Io = LanIo;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (LanIo, SocketAddr) {
        loop {
            let (stream, addr) = match self.inner.accept().await {
                Ok(got) => got,
                Err(_) => {
                    // Like axum's own listener: too many open files and the like pass.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            match keep(stream) {
                Ok((io, handle)) => {
                    let mut conns = lock(&self.conns);
                    conns.retain(|w| w.strong_count() > 0);
                    conns.push(Arc::downgrade(&handle));
                    return (LanIo { io, _handle: handle }, addr);
                }
                Err(_) => continue,
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

/// A second handle of the same socket: `shutdown` on it ends the connection for both.
fn keep(stream: TcpStream) -> io::Result<(TcpStream, Arc<std::net::TcpStream>)> {
    let std_stream = stream.into_std()?;
    let handle = std_stream.try_clone()?;
    let io = TcpStream::from_std(std_stream)?;
    let _ = io.set_nodelay(true);
    Ok((io, Arc::new(handle)))
}

/// A phone connection; while it lives, its handle is in the list of [`LanListener`].
struct LanIo {
    io: TcpStream,
    _handle: Arc<std::net::TcpStream>,
}

impl AsyncRead for LanIo {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl AsyncWrite for LanIo {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }
    fn poll_write_vectored(self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[io::IoSlice<'_>]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write_vectored(cx, bufs)
    }
    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reachable_ranges() {
        for ip in ["192.168.1.13", "10.0.0.2", "172.16.4.1", "169.254.3.3", "100.101.2.3", "127.0.0.1"] {
            assert!(reachable_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.128.0.1", "100.63.255.255", "::ffff:192.168.1.1", "fd00::1"] {
            assert!(!reachable_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(!offered("169.254.3.3".parse().unwrap()));
    }

    #[test]
    fn search_contract() {
        // Fixed with the phone app.
        assert_eq!((DISCOVERY_PORT, DISCOVER), (47823, &b"LORELAMP_DISCOVER 1"[..]));
    }

    #[test]
    fn an_address_waits_longer_each_time() {
        let secs: Vec<u64> = (1..=8).map(|n| ip_lock(n).as_secs()).collect();
        assert_eq!(secs, vec![30, 60, 120, 240, 480, 900, 900, 900]);
        assert_eq!(ip_lock(u32::MAX), IP_LOCK_MAX);
    }

    #[test]
    fn quiet_addresses_are_forgotten_but_not_while_they_wait() {
        let now = Instant::now();
        let Some(long_ago) = now.checked_sub(IP_FORGET + Duration::from_secs(1)) else { return };
        let mut senders = HashMap::new();
        let ip = |n: u8| IpAddr::from([10, 0, 0, n]);
        senders.insert(ip(1), Sender { wrong: 2, locks: 0, until: None, last: long_ago });
        senders.insert(ip(2), Sender { wrong: 0, locks: 3, until: Some(now + IP_LOCK), last: long_ago });
        senders.insert(ip(3), Sender { wrong: 1, locks: 0, until: None, last: now });
        forget_quiet(&mut senders, now);
        let mut kept: Vec<IpAddr> = senders.keys().copied().collect();
        kept.sort();
        assert_eq!(kept, vec![ip(2), ip(3)]);
    }

    #[test]
    fn codes_have_six_digits() {
        for _ in 0..200 {
            let c = new_code();
            assert_eq!(c.len(), 6);
            assert!(c.bytes().all(|b| b.is_ascii_digit()));
        }
    }
}
