//! Security headers on every answer (UI, API, manifest): a Content-Security-Policy that only lets
//! the page run its own scripts, `Referrer-Policy: no-referrer` (links to Hugging Face or a card
//! site never carry the address with its token), `nosniff`, and no framing by other pages.
//!
//! The one inline script of `index.html` (the colour theme before the first paint) is allowed by
//! its SHA-256, read from the `index.html` this server delivers, so editing the script needs no
//! hash to be kept in step by hand.

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::WebUi;

/// The policy for one server, built once at start from the UI it delivers.
#[derive(Clone)]
pub(crate) struct Policy {
    /// `'sha256-…'` for each inline script of `index.html`, space separated.
    script_hashes: String,
}

impl Policy {
    pub(crate) fn new(web: &WebUi) -> Policy {
        // The HTML parser turns CRLF and CR into LF before the browser hashes a script (a checkout
        // on Windows may carry CRLF), so the hash is taken over the same text.
        let html = web
            .file("index.html")
            .map(|b| String::from_utf8_lossy(&b).replace("\r\n", "\n").replace('\r', "\n"))
            .unwrap_or_default();
        let script_hashes = inline_scripts(&html)
            .map(|s| format!("'sha256-{}'", base64::engine::general_purpose::STANDARD.encode(Sha256::digest(s.as_bytes()))))
            .collect::<Vec<_>>()
            .join(" ");
        Policy { script_hashes }
    }

    /// The CSP for a request to `host` (its `Host` header). The socket goes to the same host:
    /// `'self'` covers `ws:` in current browsers, the explicit `ws://host` covers older WebViews.
    fn csp(&self, host: Option<&str>) -> String {
        let ws = host.filter(|h| host_chars(h)).map(|h| format!(" ws://{h}")).unwrap_or_default();
        let scripts = if self.script_hashes.is_empty() { String::new() } else { format!(" {}", self.script_hashes) };
        format!(
            "default-src 'self'; script-src 'self'{scripts}; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; media-src 'self' data: blob:; font-src 'self' data:; \
             connect-src 'self'{ws}; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; \
             form-action 'self'; frame-ancestors 'none'"
        )
    }
}

/// Only what a checked `Host` can hold (IPv4/IPv6 literal or a name, and a port): anything else
/// leaves the socket to `'self'` and can never end up inside the header.
fn host_chars(host: &str) -> bool {
    !host.is_empty() && host.len() <= 64 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b".:-[]".contains(&b))
}

/// The bodies of `<script>` elements without `src` (inline scripts), as the browser hashes them.
fn inline_scripts(html: &str) -> impl Iterator<Item = &str> {
    let mut rest = html;
    std::iter::from_fn(move || loop {
        let start = rest.find("<script")?;
        let after = &rest[start..];
        let open_end = after.find('>')?;
        let open = &after[..open_end];
        let body_start = start + open_end + 1;
        let close = rest[body_start..].find("</script>")?;
        let body = &rest[body_start..body_start + close];
        rest = &rest[body_start + close + "</script>".len()..];
        if !open.contains("src=") && !body.trim().is_empty() {
            return Some(body);
        }
    })
}

pub(crate) async fn add(State(policy): State<Policy>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&policy.csp(host.as_deref())) {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    res
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{start, Config};

    #[test]
    fn finds_inline_scripts_only() {
        let html = "<head><script>\n  var a = 1;\n</script><script type=\"module\" crossorigin src=\"/assets/i.js\"></script>\
                    </head><body><script src=\"x.js\"></script><script>b()</script></body>";
        assert_eq!(inline_scripts(html).collect::<Vec<_>>(), ["\n  var a = 1;\n", "b()"]);
        assert_eq!(inline_scripts("<p>no scripts</p>").count(), 0);
    }

    #[test]
    fn host_in_the_policy_is_plain() {
        let p = Policy { script_hashes: "'sha256-abc='".into() };
        assert!(p.csp(Some("192.168.1.20:7860")).contains("connect-src 'self' ws://192.168.1.20:7860;"));
        assert!(p.csp(Some("[::1]:7860")).contains("connect-src 'self' ws://[::1]:7860;"));
        for bad in ["a;b", "x y", "a\"b", "evil.com/'unsafe-eval'", ""] {
            assert!(p.csp(Some(bad)).contains("connect-src 'self';"), "{bad}");
        }
        assert!(p.csp(None).contains("script-src 'self' 'sha256-abc=';"));
    }

    /// Through the real server: the page and an API answer carry every header, and the hash in the
    /// policy is the one of the inline script that the delivered `index.html` holds.
    #[tokio::test]
    async fn headers_on_page_and_api() {
        let dir = std::env::temp_dir().join(format!("rp-app-headers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let web = dir.join("web");
        std::fs::create_dir_all(&web).unwrap();
        // Written with CRLF (a Windows checkout); the browser hashes the script with LF.
        let script = "\n      try { document.documentElement.dataset.theme = 'papier'; } catch (e) {}\n    ";
        let page = format!("<!doctype html><head><script>{script}</script></head><body></body>").replace('\n', "\r\n");
        std::fs::write(web.join("index.html"), page).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(web),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.join("data"),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}", running.addr.port());
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(script.as_bytes()));
        for path in ["/", "/api/library", "/chat/3"] {
            let res = reqwest::get(format!("{base}{path}")).await.unwrap();
            let h = res.headers();
            let csp = h[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
            assert!(csp.starts_with("default-src 'self'; "), "{path}: {csp}");
            assert!(csp.contains(&format!("script-src 'self' 'sha256-{hash}';")), "{path}: {csp}");
            assert!(csp.contains(&format!("connect-src 'self' ws://127.0.0.1:{};", running.addr.port())), "{path}: {csp}");
            assert!(csp.ends_with("frame-ancestors 'none'"), "{path}: {csp}");
            assert_eq!(h[header::REFERRER_POLICY], "no-referrer", "{path}");
            assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
            assert_eq!(h[header::X_FRAME_OPTIONS], "DENY", "{path}");
        }
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
