//! Fetching a web address the user pasted, for the link import's general way. The server is
//! reachable in the home network, so a pasted link must never make it talk to the PC itself or
//! to devices next to it (SSRF). Every hop is checked:
//!
//! - HTTPS on port 443 only (a plain `http://` link is tried as HTTPS, never as HTTP),
//! - the host is resolved first and **every** address must be public — no loopback, private,
//!   link-local, CGNAT, unique-local, multicast or reserved range, also not when it hides in an
//!   IPv4-mapped, NAT64, 6to4 or Teredo address — and the connection is pinned to exactly the
//!   addresses that were checked (no second DNS answer),
//! - redirects are followed by hand (at most [`MAX_REDIRECTS`]) with the same checks per hop,
//! - no proxy, no cookies, no credentials in the address, an honest user agent, a size limit
//!   and one deadline for the whole chain.
//!
//! Tests (and a test server through `RP_LINK_TEST_ORIGIN`) can exempt ONE origin, the local
//! stand-in site; everything else stays checked.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use reqwest::{redirect, StatusCode, Url};

use crate::link::{user_agent, FetchError};

/// Most redirects followed.
pub const MAX_REDIRECTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Is this an address on the public internet?
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0                                    // "this network"
        || a == 10                              // private
        || a == 127                             // loopback
        || (a == 100 && (64..128).contains(&b)) // carrier-grade NAT
        || (a == 169 && b == 254)               // link-local (and cloud metadata)
        || (a == 172 && (16..32).contains(&b))  // private
        || (a == 192 && b == 168)               // private
        || (a == 192 && b == 0 && c == 0)       // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2)       // documentation
        || (a == 192 && b == 88 && c == 99)     // 6to4 relay
        || (a == 198 && (18..20).contains(&b))  // benchmarking
        || (a == 198 && b == 51 && c == 100)    // documentation
        || (a == 203 && b == 0 && c == 113)     // documentation
        || a >= 224) // multicast, reserved, broadcast
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let s = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || s[..6] == [0, 0, 0, 0, 0, 0]          // IPv4-compatible (::a.b.c.d)
        || (s[0] & 0xfe00) == 0xfc00             // unique local fc00::/7
        || (s[0] & 0xffc0) == 0xfe80             // link-local fe80::/10
        || (s[0] & 0xffc0) == 0xfec0             // site-local (old)
        || (s[0] & 0xff00) == 0xff00             // multicast
        || (s[0] == 0x0064 && s[1] == 0xff9b)    // NAT64 (can point at any IPv4)
        || s[0] == 0x2002                        // 6to4 (embeds an IPv4)
        || (s[0] == 0x2001 && s[1] == 0)         // Teredo
        || (s[0] == 0x2001 && s[1] == 0x0db8)    // documentation
        || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0)) // discard-only
}

/// What a fetch brought.
pub struct Got {
    /// The address that answered (after redirects).
    pub url: Url,
    /// `Content-Type` without parameters, lower case.
    pub content_type: String,
    pub body: Vec<u8>,
}

/// The rules of one user action: the exempt test origin and the deadline of the whole chain.
#[derive(Clone)]
pub struct Guard {
    exempt: Option<Url>,
    deadline: Instant,
}

impl Guard {
    pub fn new(exempt: Option<Url>, budget: Duration) -> Guard {
        Guard { exempt, deadline: Instant::now() + budget }
    }

    fn is_exempt(&self, url: &Url) -> bool {
        self.exempt.as_ref().is_some_and(|e| e.origin() == url.origin())
    }

    /// Checks one hop and answers the address to request plus the addresses to pin the
    /// connection to (None: the exempt test origin).
    async fn check(&self, mut url: Url) -> Result<(Url, Option<(String, Vec<SocketAddr>)>), FetchError> {
        if self.is_exempt(&url) {
            return Ok((url, None));
        }
        if url.scheme() == "http" && url.port().is_none() {
            let _ = url.set_scheme("https");
        }
        if url.scheme() != "https" || url.port_or_known_default() != Some(443) {
            return Err(FetchError::Private);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FetchError::Private);
        }
        let host = match url.host() {
            Some(url::Host::Domain(d)) => d.to_ascii_lowercase(),
            Some(url::Host::Ipv4(ip)) => return if public_v4(ip) { Ok((url, None)) } else { Err(FetchError::Private) },
            Some(url::Host::Ipv6(ip)) => return if public_v6(ip) { Ok((url, None)) } else { Err(FetchError::Private) },
            None => return Err(FetchError::Private),
        };
        let lookup = tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host((host.as_str(), 443)));
        let addrs: Vec<SocketAddr> = match lookup.await {
            Ok(Ok(found)) => found.collect(),
            Ok(Err(_)) => return Err(FetchError::Failed),
            Err(_) => return Err(FetchError::Timeout),
        };
        if addrs.is_empty() {
            return Err(FetchError::Failed);
        }
        // One private address among them is enough to refuse: the connection could pick it.
        if !addrs.iter().all(|a| public_ip(a.ip())) {
            return Err(FetchError::Private);
        }
        Ok((url, Some((host, addrs))))
    }

    /// GET with every check; the body is at most `limit` bytes.
    pub async fn get(&self, url: Url, accept: &str, limit: usize) -> Result<Got, FetchError> {
        let mut url = url;
        for _ in 0..=MAX_REDIRECTS {
            let left = self.deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(FetchError::Timeout);
            }
            let (checked, pin) = self.check(url).await?;
            url = checked;
            let mut builder = reqwest::Client::builder()
                .user_agent(user_agent())
                .redirect(redirect::Policy::none())
                .no_proxy()
                .timeout(left)
                .connect_timeout(CONNECT_TIMEOUT.min(left))
                .https_only(!self.is_exempt(&url));
            if let Some((host, addrs)) = &pin {
                builder = builder.resolve_to_addrs(host, addrs);
            }
            let client = builder.build().map_err(|_| FetchError::Failed)?;
            let failed = |e: reqwest::Error| if e.is_timeout() { FetchError::Timeout } else { FetchError::Failed };
            let mut res = client.get(url.clone()).header("accept", accept).send().await.map_err(failed)?;
            let status = res.status();
            if status.is_redirection() {
                let to = res.headers().get("location").and_then(|v| v.to_str().ok()).ok_or(FetchError::Redirect)?;
                let mut next = url.join(to).map_err(|_| FetchError::Redirect)?;
                next.set_fragment(None);
                url = next;
                continue;
            }
            // A bot check (Cloudflare and others) is never worked around.
            if res.headers().contains_key("cf-mitigated") {
                return Err(FetchError::Blocked);
            }
            match status {
                s if s.is_success() => {}
                StatusCode::NOT_FOUND | StatusCode::GONE => return Err(FetchError::NotFound),
                StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED | StatusCode::SERVICE_UNAVAILABLE => return Err(FetchError::Blocked),
                StatusCode::TOO_MANY_REQUESTS => return Err(FetchError::RateLimited),
                _ => return Err(FetchError::Failed),
            }
            if res.content_length().is_some_and(|n| n > limit as u64) {
                return Err(FetchError::TooLarge);
            }
            let content_type = res
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
                .unwrap_or_default();
            let mut body = Vec::new();
            while let Some(chunk) = res.chunk().await.map_err(failed)? {
                if body.len() + chunk.len() > limit {
                    return Err(FetchError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            return Ok(Got { url, content_type, body });
        }
        Err(FetchError::Redirect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn private_ranges_are_not_public() {
        for bad in [
            "127.0.0.1", "127.8.9.1", "10.0.0.1", "10.255.255.255", "172.16.0.1", "172.31.255.1", "192.168.0.1", "192.168.178.20",
            "169.254.169.254", "100.64.0.1", "0.0.0.0", "224.0.0.1", "255.255.255.255", "198.18.0.1", "192.0.2.1",
            "::1", "::", "fc00::1", "fd12:3456::1", "fe80::1", "ff02::1", "::ffff:127.0.0.1", "::ffff:192.168.1.1", "::10.0.0.1",
            "64:ff9b::7f00:1", "2002:7f00:1::1", "2001:0:1::1", "2001:db8::1", "fec0::1",
        ] {
            assert!(!public_ip(ip(bad)), "{bad}");
        }
        for good in ["1.1.1.1", "8.8.8.8", "172.15.0.1", "172.32.0.1", "100.63.0.1", "100.128.0.1", "2606:4700:4700::1111", "2a00:1450:4001::1", "::ffff:8.8.8.8"] {
            assert!(public_ip(ip(good)), "{good}");
        }
    }

    async fn refused(link: &str) -> FetchError {
        let guard = Guard::new(None, Duration::from_secs(5));
        match guard.get(Url::parse(link).unwrap(), "*/*", 1024).await {
            Err(e) => e,
            Ok(_) => panic!("{link} was fetched"),
        }
    }

    #[tokio::test]
    async fn local_addresses_and_odd_ports_are_refused_before_any_request() {
        for link in [
            "https://127.0.0.1/card.png",
            "http://127.0.0.1/card.png",
            "https://10.0.0.5/card.png",
            "https://192.168.1.1/card.png",
            "https://169.254.169.254/latest/meta-data/",
            "https://[::1]/card.png",
            "https://[fd00::1]/card.png",
            "https://[::ffff:10.0.0.1]/card.png",
            // Resolved first: the name points at this PC.
            "https://localhost/card.png",
            // Not a standard port, credentials in the address.
            "https://example.com:8443/card.png",
            "http://example.com:8080/card.png",
            "https://user:pw@example.com/card.png",
        ] {
            assert_eq!(refused(link).await, FetchError::Private, "{link}");
        }
    }
}
