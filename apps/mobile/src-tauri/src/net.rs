//! The three things the app does on the network before it opens the PC's interface: ask whether
//! Lorelamp answers at an address, trade a pairing code for a token, and look for PCs by UDP
//! broadcast.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use url::Host;

use crate::link::{self, Fail, Found, Link};

/// Loopback addresses count as "local" only in debug builds (tests over `adb reverse`).
pub const LOOPBACK: bool = cfg!(debug_assertions);

fn client() -> Result<reqwest::Client, Fail> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| Fail::Unreachable)
}

/// A host name is only used when everything it resolves to lies in the local network.
async fn name_stays_local(host: &Host<String>, port: u16) -> Result<(), Fail> {
    let Host::Domain(name) = host else { return Ok(()) };
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), port)).await.map_err(|_| Fail::Unreachable)?.collect();
    if addrs.is_empty() {
        return Err(Fail::Unreachable);
    }
    if addrs.iter().all(|a| link::ip_local(a.ip(), LOOPBACK)) {
        Ok(())
    } else {
        Err(Fail::NotLocal)
    }
}

/// Does Lorelamp answer at this address, and does it take the token?
pub async fn check(link: &Link) -> Result<(), Fail> {
    name_stays_local(&link.host, link.port).await?;
    let res = client()?
        .get(format!("{}/api/health", link.origin()))
        .header("authorization", format!("Bearer {}", link.token))
        .send()
        .await
        .map_err(|_| Fail::Unreachable)?;
    match res.status().as_u16() {
        200 => {
            let body = res.text().await.map_err(|_| Fail::NotLorelamp)?;
            let ok = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("ok")?.as_bool()) == Some(true);
            if ok { Ok(()) } else { Err(Fail::NotLorelamp) }
        }
        401 => Err(Fail::TokenRejected),
        _ => Err(Fail::NotLorelamp),
    }
}

/// Trades the six-digit code shown at the PC for the address with its token.
pub async fn pair(host: Host<String>, port: u16, code: &str) -> Result<Link, Fail> {
    name_stays_local(&host, port).await?;
    let res = client()?
        .post(format!("http://{host}:{port}/api/pair"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "code": code }).to_string())
        .send()
        .await
        .map_err(|_| Fail::Unreachable)?;
    let status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    let token = link::pair_token(status, &body)?;
    Ok(Link { host, port, token })
}

/// Where a broadcast has to go so that every PC in this network hears it: the limited broadcast
/// address and the broadcast address of every network the phone is in.
fn broadcast_targets() -> BTreeSet<Ipv4Addr> {
    let mut targets = BTreeSet::from([Ipv4Addr::BROADCAST]);
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        if let if_addrs::IfAddr::V4(v4) = iface.addr {
            if v4.ip.is_loopback() {
                continue;
            }
            let (ip, mask) = (u32::from(v4.ip), u32::from(v4.netmask));
            targets.insert(v4.broadcast.unwrap_or(Ipv4Addr::from(ip | !mask)));
        }
    }
    targets
}

/// Sends the search datagram and collects the answers of `wait`. Blocking.
pub fn discover(wait: Duration) -> Vec<Found> {
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return Vec::new() };
    let _ = socket.set_broadcast(true);
    let mut sent = false;
    for target in broadcast_targets() {
        sent |= socket.send_to(link::DISCOVER_MESSAGE, (target, link::DISCOVER_PORT)).is_ok();
    }
    if !sent {
        return Vec::new();
    }
    let end = Instant::now() + wait;
    let mut found: Vec<Found> = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() || socket.set_read_timeout(Some(left)).is_err() {
            break;
        }
        let Ok((n, from)) = socket.recv_from(&mut buf) else { break };
        let sender: IpAddr = from.ip();
        if let Some(pc) = link::parse_reply(&buf[..n], sender, LOOPBACK) {
            if !found.iter().any(|f| f.addr == pc.addr) {
                found.push(pc);
            }
        }
    }
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.addr.cmp(&b.addr)));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// One real HTTP exchange on a loopback socket: answers the next request with `status` and
    /// `body` and hands back what was asked.
    fn answer_once(status: &'static str, body: &'static str) -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 2048];
            loop {
                let n = s.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).to_string();
                let Some(head_end) = text.find("\r\n\r\n") else { continue };
                let len = text
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                if req.len() >= head_end + 4 + len || n == 0 {
                    break;
                }
            }
            write!(s, "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&req).to_string()
        });
        (port, handle)
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    fn local(port: u16) -> Link {
        Link { host: Host::Ipv4(Ipv4Addr::LOCALHOST), port, token: "tok".into() }
    }

    #[test]
    fn check_reads_the_answer() {
        let (port, asked) = answer_once("200 OK", r#"{"ok":true}"#);
        assert_eq!(rt().block_on(check(&local(port))), Ok(()));
        let req = asked.join().unwrap();
        assert!(req.starts_with("GET /api/health HTTP/1.1"), "{req}");
        assert!(req.to_ascii_lowercase().contains("authorization: bearer tok"), "{req}");

        let (port, _) = answer_once("401 Unauthorized", "missing or wrong token");
        assert_eq!(rt().block_on(check(&local(port))), Err(Fail::TokenRejected));
        let (port, _) = answer_once("200 OK", r#"{"hello":"world"}"#);
        assert_eq!(rt().block_on(check(&local(port))), Err(Fail::NotLorelamp));
        let (port, _) = answer_once("404 Not Found", "");
        assert_eq!(rt().block_on(check(&local(port))), Err(Fail::NotLorelamp));
    }

    #[test]
    fn check_nothing_there() {
        // A port nobody listens on: bound once to get a free number, then closed again.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(rt().block_on(check(&local(port))), Err(Fail::Unreachable));
    }

    #[test]
    fn pair_sends_the_code_and_reads_the_token() {
        let (port, asked) = answer_once("200 OK", r#"{"token":"fresh"}"#);
        let link = rt().block_on(pair(Host::Ipv4(Ipv4Addr::LOCALHOST), port, "123456")).unwrap();
        assert_eq!(link.url(), format!("http://127.0.0.1:{port}/?token=fresh"));
        let req = asked.join().unwrap();
        assert!(req.starts_with("POST /api/pair HTTP/1.1"), "{req}");
        assert!(req.ends_with(r#"{"code":"123456"}"#), "{req}");

        for (status, body, why) in [
            ("401 Unauthorized", r#"{"error":"wrong_code"}"#, Fail::WrongCode),
            ("429 Too Many Requests", r#"{"error":"too_many"}"#, Fail::TooMany),
            ("409 Conflict", r#"{"error":"lan_off"}"#, Fail::LanOff),
        ] {
            let (port, _) = answer_once(status, body);
            assert_eq!(rt().block_on(pair(Host::Ipv4(Ipv4Addr::LOCALHOST), port, "123456")), Err(why));
        }
    }

    #[test]
    fn broadcast_always_includes_the_limited_address() {
        assert!(broadcast_targets().contains(&Ipv4Addr::BROADCAST));
    }
}
