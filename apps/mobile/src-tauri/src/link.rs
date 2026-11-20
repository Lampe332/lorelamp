//! What the app accepts as "Lorelamp on a PC": the address from the QR code, a typed `IP:Port`,
//! a search reply and the answer to a pairing code. Pure checks, no I/O.
//!
//! The app talks plain HTTP (a home network has no certificates), so every address must lie in
//! the local network, and exactly where the PC's server lets phones in (`crates/server`:
//! `lan::reachable_ip` + `guard::host_allowed`): an IPv4 literal that is private, link-local or in
//! the shared range VPN meshes use (100.64.0.0/10). The server's phone listener is IPv4 only and
//! refuses host names (DNS rebinding), so the app takes neither. Loopback (`127.0.0.1`,
//! `localhost`, `[::1]`) is accepted in debug builds only (tests over `adb reverse`).

use std::net::{IpAddr, Ipv4Addr};

use serde::Serialize;
use url::{Host, Url};

/// Why a step failed; the start page turns the code into a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fail {
    /// Not an address of Lorelamp at all (other scheme, no host, nonsense).
    NotALink,
    /// An `https://` address: Lorelamp on a PC never has one.
    NotHttp,
    /// The address lies outside the local network.
    NotLocal,
    /// The address carries no (usable) token.
    NoToken,
    /// The typed `IP:Port` cannot be read.
    BadAddress,
    /// The typed code is not six digits.
    BadCode,
    /// Nothing answers at the address.
    Unreachable,
    /// Something answers, but it is not Lorelamp.
    NotLorelamp,
    /// Lorelamp answers and refuses the token (it was renewed at the PC).
    TokenRejected,
    /// Pairing: the code is wrong.
    WrongCode,
    /// Pairing: too many wrong codes, the PC wants a new one.
    TooMany,
    /// Pairing: "Handy verbinden" is switched off at the PC.
    LanOff,
    /// No connection is stored.
    NoneSaved,
    /// The connection could not be stored on the phone.
    Storage,
}

/// A checked address of Lorelamp on a PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub host: Host<String>,
    pub port: u16,
    pub token: String,
}

impl Link {
    /// `http://host:port`, IPv6 in brackets.
    pub fn origin(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// `host:port`, as the start page shows a PC.
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// The address as the PC shows it in its QR code; this is what gets stored.
    pub fn url(&self) -> String {
        format!("{}/?token={}", self.origin(), self.token)
    }

    /// The address the WebView opens: the PC's interface, told that it runs inside this app.
    pub fn open_url(&self) -> String {
        format!("{}&shell=android", self.url())
    }
}

/// The shared address range of carrier NAT and VPN meshes (100.64.0.0/10), which the PC offers in
/// its QR code too.
fn shared_range(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    a == 100 && (64..128).contains(&b)
}

/// Is this address one the PC lets phones in from? IPv6 only as loopback (debug builds): the
/// phone listener has no IPv6 socket.
pub fn ip_local(ip: IpAddr, loopback: bool) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || shared_range(v4) || (loopback && v4.is_loopback()),
        IpAddr::V6(v6) => loopback && v6.is_loopback(),
    }
}

fn host_local(host: &Host<String>, loopback: bool) -> bool {
    match host {
        Host::Ipv4(ip) => ip_local(IpAddr::V4(*ip), loopback),
        Host::Ipv6(ip) => ip_local(IpAddr::V6(*ip), loopback),
        // The server answers a name with 403 "forbidden host"; `localhost` is loopback.
        Host::Domain(name) => loopback && name.eq_ignore_ascii_case("localhost"),
    }
}

/// A token as the server makes them: URL-safe characters only, so it never needs escaping.
fn token_ok(token: &str) -> bool {
    (1..=256).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

/// Checks a scanned or passed address: `http://<local host>:<port>/?token=<token>`.
pub fn parse_link(raw: &str, loopback: bool) -> Result<Link, Fail> {
    let url = Url::parse(raw.trim()).map_err(|_| Fail::NotALink)?;
    match url.scheme() {
        "http" => {}
        "https" => return Err(Fail::NotHttp),
        _ => return Err(Fail::NotALink),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Fail::NotALink);
    }
    let host = url.host().ok_or(Fail::NotALink)?.to_owned();
    if !host_local(&host, loopback) {
        return Err(Fail::NotLocal);
    }
    let port = url.port_or_known_default().ok_or(Fail::NotALink)?;
    let token = url.query_pairs().find(|(k, _)| k == "token").map(|(_, v)| v.into_owned()).ok_or(Fail::NoToken)?;
    if !token_ok(&token) {
        return Err(Fail::NoToken);
    }
    Ok(Link { host, port, token })
}

/// `lorelamp://connect?u=<url-encoded address>` → the address. Anything else is `None`.
pub fn deep_link_target(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    if url.scheme() != "lorelamp" || url.host_str() != Some("connect") {
        return None;
    }
    url.query_pairs().find(|(k, _)| k == "u").map(|(_, v)| v.into_owned())
}

/// What the start page asks before an address handed in from outside is used: a link can come
/// from any web page or app on the phone, so it never connects or replaces the remembered PC on
/// its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offer {
    /// `host:port` of the PC the link points to.
    pub target: String,
    /// `host:port` of the remembered PC that a yes replaces (none when nothing is remembered or it
    /// is the same PC).
    pub replaces: Option<String>,
}

/// Checks `lorelamp://connect?u=…` exactly like a scanned code and says what a yes would do.
pub fn offer(deep_link: &str, saved: Option<&Link>, loopback: bool) -> Result<(Link, Offer), Fail> {
    let raw = deep_link_target(deep_link).ok_or(Fail::NotALink)?;
    let link = parse_link(&raw, loopback)?;
    let replaces = saved.filter(|s| s.origin() != link.origin()).map(Link::addr);
    let offer = Offer { target: link.addr(), replaces };
    Ok((link, offer))
}

/// A typed `IP:Port` (as shown at the PC under the code). A pasted `http://` in front is fine.
pub fn parse_addr(raw: &str, loopback: bool) -> Result<(Host<String>, u16), Fail> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("http://").unwrap_or(raw).trim_end_matches('/');
    if raw.is_empty() || raw.contains(['/', '?', '#', '@', ' ']) {
        return Err(Fail::BadAddress);
    }
    let url = Url::parse(&format!("http://{raw}/")).map_err(|_| Fail::BadAddress)?;
    let host = url.host().ok_or(Fail::BadAddress)?.to_owned();
    // The port has to be typed: Lorelamp never listens on 80.
    let port = url.port().ok_or(Fail::BadAddress)?;
    if !host_local(&host, loopback) {
        return Err(Fail::NotLocal);
    }
    Ok((host, port))
}

/// The six digits of a pairing code; spaces between them are fine.
pub fn parse_code(raw: &str) -> Result<String, Fail> {
    let code: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()) {
        Ok(code)
    } else {
        Err(Fail::BadCode)
    }
}

/// What to send into the network to find Lorelamp, and where it listens.
pub const DISCOVER_PORT: u16 = 47823;
pub const DISCOVER_MESSAGE: &[u8] = b"LORELAMP_DISCOVER 1";

/// A PC that answered the search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub name: String,
    /// `ip:port` of its interface.
    pub addr: String,
}

/// Reads a search reply `{"app":"lorelamp","v":1,"name":"…","port":…}` from `sender`.
pub fn parse_reply(data: &[u8], sender: IpAddr, loopback: bool) -> Option<Found> {
    let v: serde_json::Value = serde_json::from_slice(data).ok()?;
    if v.get("app")?.as_str()? != "lorelamp" || v.get("v")?.as_u64()? != 1 {
        return None;
    }
    let port = u16::try_from(v.get("port")?.as_u64()?).ok().filter(|p| *p != 0)?;
    // A dual-stack socket reports an IPv4 sender as `::ffff:a.b.c.d`; the PC is reached at a.b.c.d.
    let sender = match sender {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(sender, IpAddr::V4),
        v4 => v4,
    };
    if !ip_local(sender, loopback) {
        return None;
    }
    // A name is only shown, never trusted: keep what can be printed, and not too much of it.
    let name: String = v.get("name").and_then(|n| n.as_str()).unwrap_or("").chars().filter(|c| !c.is_control()).take(48).collect();
    let name = if name.trim().is_empty() { "PC".to_string() } else { name.trim().to_string() };
    let host = match sender {
        IpAddr::V4(ip) => Host::<String>::Ipv4(ip),
        IpAddr::V6(ip) => Host::Ipv6(ip),
    };
    Some(Found { name, addr: format!("{host}:{port}") })
}

/// Reads the answer of `POST /api/pair`: 200 `{"token":"…"}`, else why not.
pub fn pair_token(status: u16, body: &str) -> Result<String, Fail> {
    match status {
        200 => serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("token")?.as_str().map(str::to_owned))
            .filter(|t| token_ok(t))
            .ok_or(Fail::NotLorelamp),
        401 => Err(Fail::WrongCode),
        429 => Err(Fail::TooMany),
        409 => Err(Fail::LanOff),
        _ => Err(Fail::NotLorelamp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> Link {
        parse_link(raw, false).unwrap_or_else(|e| panic!("{raw}: {e:?}"))
    }

    #[test]
    fn accepts_addresses_in_the_home_network() {
        let l = ok("http://192.168.178.20:8765/?token=abcDEF123");
        assert_eq!((l.host.to_string().as_str(), l.port, l.token.as_str()), ("192.168.178.20", 8765, "abcDEF123"));
        assert_eq!(l.url(), "http://192.168.178.20:8765/?token=abcDEF123");
        assert_eq!(l.open_url(), "http://192.168.178.20:8765/?token=abcDEF123&shell=android");
        for raw in [
            "http://10.0.0.5:8765/?token=t",
            "http://172.16.4.1:8765/?token=t",
            "http://172.31.255.254:8765/?token=t",
            "http://169.254.10.20:8765/?token=t",
            "http://100.64.0.1:8765/?token=t",
            "http://100.127.255.254:8765/?token=t",
            "  http://192.168.1.2:8765/?x=1&token=t  ",
        ] {
            ok(raw);
        }
    }

    /// The same ranges as the server's `lan::reachable_ip` test (`crates/server/src/lan.rs`), so a
    /// QR code the PC shows is always taken and the app never offers what the PC refuses.
    #[test]
    fn same_ranges_as_the_server() {
        for ip in ["192.168.1.13", "10.0.0.2", "172.16.4.1", "169.254.3.3", "100.101.2.3"] {
            assert!(ip_local(ip.parse().unwrap(), false), "{ip}");
        }
        for ip in ["8.8.8.8", "100.128.0.1", "100.63.255.255", "::ffff:192.168.1.1", "fd00::1", "fe80::1", "127.0.0.1", "::1"] {
            assert!(!ip_local(ip.parse().unwrap(), false), "{ip}");
        }
        // The server answers 403 "forbidden host" to names and has no IPv6 phone listener.
        for raw in [
            "http://wohnzimmer-pc:8765/?token=t",
            "http://desktop.local:8765/?token=t",
            "http://desktop.fritz.box:8765/?token=t",
            "http://pc.home.arpa:8765/?token=t",
            "http://[fd12:3456:789a::1]:8765/?token=t",
            "http://[fe80::1]:8765/?token=t",
            "http://[::ffff:192.168.1.2]:8765/?token=t",
        ] {
            assert_eq!(parse_link(raw, false), Err(Fail::NotLocal), "{raw}");
            assert_eq!(parse_link(raw, true), Err(Fail::NotLocal), "{raw}");
        }
    }

    #[test]
    fn refuses_everything_else() {
        for (raw, why) in [
            ("http://8.8.8.8:8765/?token=t", Fail::NotLocal),
            ("http://172.32.0.1:8765/?token=t", Fail::NotLocal),
            ("http://100.128.0.1:8765/?token=t", Fail::NotLocal),
            ("http://[2001:db8::1]:8765/?token=t", Fail::NotLocal),
            ("http://[::ffff:8.8.8.8]:8765/?token=t", Fail::NotLocal),
            ("http://example.com:8765/?token=t", Fail::NotLocal),
            ("http://192.168.1.2.example.com:8765/?token=t", Fail::NotLocal),
            ("https://example.com/?token=t", Fail::NotHttp),
            ("https://192.168.1.2:8765/?token=t", Fail::NotHttp),
            ("http://192.168.1.2:8765/", Fail::NoToken),
            ("http://192.168.1.2:8765/?token=", Fail::NoToken),
            ("http://192.168.1.2:8765/?token=a%20b", Fail::NoToken),
            ("http://192.168.1.2:8765/?tokens=t", Fail::NoToken),
            ("ftp://192.168.1.2:8765/?token=t", Fail::NotALink),
            ("javascript:alert(1)", Fail::NotALink),
            ("lorelamp://connect?u=x", Fail::NotALink),
            ("WIFI:S:home;T:WPA;P:secret;;", Fail::NotALink),
            ("http://192.168.1.2@example.com:8765/?token=t", Fail::NotALink),
            ("192.168.1.2:8765", Fail::NotALink),
            ("", Fail::NotALink),
        ] {
            assert_eq!(parse_link(raw, false), Err(why), "{raw}");
        }
    }

    #[test]
    fn loopback_only_for_tests() {
        for raw in ["http://127.0.0.1:8868/?token=t", "http://localhost:8868/?token=t", "http://[::1]:8868/?token=t"] {
            assert_eq!(parse_link(raw, false), Err(Fail::NotLocal), "{raw}");
            assert!(parse_link(raw, true).is_ok(), "{raw}");
        }
    }

    #[test]
    fn deep_link() {
        assert_eq!(
            deep_link_target("lorelamp://connect?u=http%3A%2F%2F192.168.1.2%3A8765%2F%3Ftoken%3Dabc").as_deref(),
            Some("http://192.168.1.2:8765/?token=abc")
        );
        assert_eq!(deep_link_target("lorelamp://other?u=x"), None);
        assert_eq!(deep_link_target("lorelamp://connect"), None);
        assert_eq!(deep_link_target("http://connect?u=x"), None);
    }

    const DEEP: &str = "lorelamp://connect?u=http%3A%2F%2F192.168.1.2%3A8765%2F%3Ftoken%3Dabc";

    #[test]
    fn deep_link_is_only_an_offer() {
        // Nothing remembered: the target, nothing replaced.
        let (link, o) = offer(DEEP, None, false).unwrap();
        assert_eq!(link.url(), "http://192.168.1.2:8765/?token=abc");
        assert_eq!(o, Offer { target: "192.168.1.2:8765".into(), replaces: None });
        // Another PC remembered: a yes replaces it, and the question says which one.
        let saved = ok("http://192.168.1.50:7860/?token=old");
        assert_eq!(offer(DEEP, Some(&saved), false).unwrap().1.replaces.as_deref(), Some("192.168.1.50:7860"));
        // The same PC (only a new token): nothing is replaced.
        let same = ok("http://192.168.1.2:8765/?token=old");
        assert_eq!(offer(DEEP, Some(&same), false).unwrap().1.replaces, None);
    }

    #[test]
    fn deep_link_is_checked_like_a_scan() {
        let wrap = |u: &str| format!("lorelamp://connect?u={}", url::form_urlencoded::byte_serialize(u.as_bytes()).collect::<String>());
        for (inner, why) in [
            ("http://8.8.8.8:8765/?token=t", Fail::NotLocal),
            ("http://evil.example:8765/?token=t", Fail::NotLocal),
            ("http://pc.local:8765/?token=t", Fail::NotLocal),
            ("http://[fd12::1]:8765/?token=t", Fail::NotLocal),
            ("https://192.168.1.2:8765/?token=t", Fail::NotHttp),
            ("http://192.168.1.2:8765/", Fail::NoToken),
            ("http://127.0.0.1:8765/?token=t", Fail::NotLocal),
            ("javascript:alert(1)", Fail::NotALink),
        ] {
            assert_eq!(offer(&wrap(inner), None, false).map(|(_, o)| o), Err(why), "{inner}");
        }
        assert_eq!(offer("lorelamp://other?u=x", None, false).map(|(_, o)| o), Err(Fail::NotALink));
        assert_eq!(offer("http://192.168.1.2:8765/?token=t", None, false).map(|(_, o)| o), Err(Fail::NotALink));
        assert!(offer(&wrap("http://127.0.0.1:8765/?token=t"), None, true).is_ok());
    }

    #[test]
    fn typed_address() {
        assert_eq!(parse_addr(" 192.168.1.2:8765 ", false).map(|(h, p)| (h.to_string(), p)), Ok(("192.168.1.2".into(), 8765)));
        assert_eq!(parse_addr("http://192.168.1.2:8765/", false).map(|(_, p)| p), Ok(8765));
        assert_eq!(parse_addr("100.100.1.2:7860", false).map(|(h, _)| h.to_string()), Ok("100.100.1.2".into()));
        assert_eq!(parse_addr("[fd12::1]:8765", false), Err(Fail::NotLocal));
        assert_eq!(parse_addr("pc.local:8765", false), Err(Fail::NotLocal));
        for raw in ["192.168.1.2", "192.168.1.2:", "192.168.1.2:99999", "192.168.1.2:8765/x", "a b:1", "", "x@192.168.1.2:1"] {
            assert_eq!(parse_addr(raw, false), Err(Fail::BadAddress), "{raw}");
        }
        assert_eq!(parse_addr("8.8.8.8:8765", false), Err(Fail::NotLocal));
        assert_eq!(parse_addr("127.0.0.1:8868", false), Err(Fail::NotLocal));
        assert!(parse_addr("127.0.0.1:8868", true).is_ok());
    }

    #[test]
    fn pairing_code() {
        assert_eq!(parse_code("123456").as_deref(), Ok("123456"));
        assert_eq!(parse_code(" 123 456 ").as_deref(), Ok("123456"));
        for raw in ["", "12345", "1234567", "12345a", "１２３４５６"] {
            assert_eq!(parse_code(raw), Err(Fail::BadCode), "{raw}");
        }
    }

    #[test]
    fn search_reply() {
        let from: IpAddr = "192.168.1.2".parse().unwrap();
        let found = parse_reply(br#"{"app":"lorelamp","v":1,"name":"Wohnzimmer","port":8765}"#, from, false);
        assert_eq!(found, Some(Found { name: "Wohnzimmer".into(), addr: "192.168.1.2:8765".into() }));
        let unnamed = parse_reply(br#"{"app":"lorelamp","v":1,"port":8765}"#, from, false).unwrap();
        assert_eq!(unnamed.name, "PC");
        for bad in [
            &br#"{"app":"other","v":1,"name":"x","port":8765}"#[..],
            br#"{"app":"lorelamp","v":2,"name":"x","port":8765}"#,
            br#"{"app":"lorelamp","v":1,"name":"x","port":0}"#,
            br#"{"app":"lorelamp","v":1,"name":"x","port":70000}"#,
            br#"{"app":"lorelamp","v":1,"name":"x"}"#,
            b"LORELAMP_DISCOVER 1",
            b"",
        ] {
            assert_eq!(parse_reply(bad, from, false), None);
        }
        let good = br#"{"app":"lorelamp","v":1,"name":"x","port":8765}"#;
        assert_eq!(parse_reply(good, "8.8.8.8".parse().unwrap(), false), None);
        assert_eq!(parse_reply(good, "fd12::1".parse().unwrap(), false), None);
        assert_eq!(parse_reply(good, "::ffff:192.168.1.2".parse().unwrap(), false).unwrap().addr, "192.168.1.2:8765");
        assert_eq!(parse_reply(good, "100.70.1.2".parse().unwrap(), false).unwrap().addr, "100.70.1.2:8765");
        let long = format!(r#"{{"app":"lorelamp","v":1,"name":"{}","port":1}}"#, "a".repeat(200));
        assert_eq!(parse_reply(long.as_bytes(), from, false).unwrap().name.len(), 48);
    }

    #[test]
    fn pairing_answer() {
        assert_eq!(pair_token(200, r#"{"token":"abc123"}"#).as_deref(), Ok("abc123"));
        assert_eq!(pair_token(200, r#"{"token":""}"#), Err(Fail::NotLorelamp));
        assert_eq!(pair_token(200, r#"{"token":"a b"}"#), Err(Fail::NotLorelamp));
        assert_eq!(pair_token(200, "<html>"), Err(Fail::NotLorelamp));
        assert_eq!(pair_token(401, r#"{"error":"wrong_code"}"#), Err(Fail::WrongCode));
        assert_eq!(pair_token(429, r#"{"error":"too_many"}"#), Err(Fail::TooMany));
        assert_eq!(pair_token(409, r#"{"error":"lan_off"}"#), Err(Fail::LanOff));
        assert_eq!(pair_token(404, ""), Err(Fail::NotLorelamp));
    }
}
