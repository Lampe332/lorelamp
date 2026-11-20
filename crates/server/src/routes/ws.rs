//! `WS /ws`: one socket per device. The client opens a chat (`open`), sends (`send`) and stops
//! (`stop`); the server sends that chat's `history` snapshot, the live events of the model, and
//! `{"type":"library"}` whenever characters, chats or settings change.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap};
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::chat::{self, Hub};
use crate::logs::{Entry, Level};
use crate::reqlog::device_of;
use crate::AppState;

/// The largest message a device may send over its socket (axum allows 64 MiB). A `send` with the
/// longest text the hub takes (`MESSAGE_MAX_CHARS`, even fully escaped as JSON) fits; such a
/// text is answered with the notice `too_long`, anything beyond this closes the socket.
const MAX_FRAME: usize = 1 << 20;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    /// Show this chat: answered with its `history` snapshot, then live events.
    Open { chat_id: i64 },
    Send { chat_id: i64, text: String },
    Stop,
}

pub(super) async fn ws_handler(
    ws: WebSocketUpgrade,
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let device = device_of(Some(&addr), &headers);
    let ua: String = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").chars().take(200).collect();
    ws.max_message_size(MAX_FRAME).max_frame_size(MAX_FRAME).on_upgrade(move |socket| async move {
        let log = st.log().clone();
        log.write(Entry::new(Level::Info, "server", "device", "device connected").device(device.clone()).data(json!({ "user_agent": ua })));
        let started = std::time::Instant::now();
        ws_session(socket, st.hub).await;
        log.write(
            Entry::new(Level::Info, "server", "device", "device disconnected")
                .device(device)
                .data(json!({ "seconds": started.elapsed().as_secs() })),
        );
    })
}

async fn ws_session(mut socket: WebSocket, hub: Arc<Hub>) {
    let mut rx = hub.subscribe();
    let mut current: Option<i64> = None;
    loop {
        // Some(chat) = (re)send that chat's snapshot and continue from a fresh subscription.
        let mut open: Option<i64> = None;
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(json) => {
                    if socket.send(Message::text(json)).await.is_err() { break; }
                }
                Err(RecvError::Lagged(_)) => {
                    // Too slow: events were dropped. Resync what this client is looking at.
                    rx = hub.subscribe();
                    if socket.send(Message::text(json!({ "type": "library" }).to_string())).await.is_err() { break; }
                    open = current;
                }
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMsg>(&t) {
                    Ok(ClientMsg::Open { chat_id }) => open = Some(chat_id),
                    Ok(ClientMsg::Send { chat_id, text }) => {
                        if let Err(code) = hub.send(chat_id, text) {
                            if socket.send(Message::text(chat::notice(code))).await.is_err() { break; }
                        }
                    }
                    Ok(ClientMsg::Stop) => hub.stop(),
                    Err(_) => {}
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
        if let Some(chat_id) = open {
            let reply = match hub.open_view(chat_id) {
                Ok(Some((snapshot, new_rx))) => {
                    rx = new_rx;
                    current = Some(chat_id);
                    snapshot
                }
                Ok(None) => {
                    current = None;
                    chat::notice("chat_missing")
                }
                Err(e) => {
                    eprintln!("storage error: {e}");
                    chat::notice("storage")
                }
            };
            if socket.send(Message::text(reply)).await.is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Overlong messages through the real server: over the socket and over HTTP.

    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use crate::chat::MESSAGE_MAX_CHARS;
    use crate::store::tests::{sample, temp_store};
    use crate::{start, Config, WebUi};

    /// A WebSocket client on a plain TCP stream (the handshake, masked frames out, frames in).
    struct Socket(TcpStream);

    impl Socket {
        async fn open(port: u16, token: &str) -> Socket {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let hello = format!(
                "GET /api/ws?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
            );
            s.write_all(hello.as_bytes()).await.unwrap();
            let mut head = vec![];
            while !head.ends_with(b"\r\n\r\n") {
                head.push(s.read_u8().await.unwrap());
            }
            assert!(head.starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head));
            Socket(s)
        }

        /// Sends one text frame (a client masks it; the mask here is random enough for a test).
        async fn send(&mut self, text: &str) {
            let data = text.as_bytes();
            let mut frame = vec![0x81u8];
            match data.len() {
                n if n < 126 => frame.push(0x80 | n as u8),
                n if n <= 0xFFFF => {
                    frame.push(0x80 | 126);
                    frame.extend_from_slice(&(n as u16).to_be_bytes());
                }
                n => {
                    frame.push(0x80 | 127);
                    frame.extend_from_slice(&(n as u64).to_be_bytes());
                }
            }
            let mask = [0x12u8, 0x34, 0x56, 0x78];
            frame.extend_from_slice(&mask);
            frame.extend(data.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
            // The server may close the socket before it read everything (too large).
            let _ = self.0.write_all(&frame).await;
        }

        /// The next frame from the server: (opcode, payload); None when the connection ended.
        async fn next(&mut self) -> Option<(u8, Vec<u8>)> {
            let read = async {
                let b0 = self.0.read_u8().await.ok()?;
                let len = match self.0.read_u8().await.ok()? & 0x7F {
                    126 => self.0.read_u16().await.ok()? as usize,
                    127 => self.0.read_u64().await.ok()? as usize,
                    n => n as usize,
                };
                let mut payload = vec![0; len];
                self.0.read_exact(&mut payload).await.ok()?;
                Some((b0 & 0x0F, payload))
            };
            tokio::time::timeout(Duration::from_secs(10), read).await.expect("the server answers")
        }

        /// The next `notice` the server sends; None: it closed the socket instead.
        async fn notice(&mut self) -> Option<String> {
            loop {
                match self.next().await? {
                    (0x1, text) => {
                        let ev: Value = serde_json::from_slice(&text).unwrap();
                        if ev["type"] == "notice" {
                            return ev["code"].as_str().map(str::to_string);
                        }
                    }
                    (0x8, _) => return None,
                    _ => {}
                }
            }
        }
    }

    #[tokio::test]
    async fn overlong_messages_are_refused_over_socket_and_http() {
        let (mut store, dir) = temp_store();
        let character = store.add_character(&sample(r#"{"name":"Mara","first_mes":"Hi."}"#), None).unwrap().id;
        let chat = store.create_chat(character, Some("Hi.")).unwrap();
        let greeting = store.messages(chat).unwrap()[0].id;
        drop(store);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        let running = start(Config {
            port: 0,
            lan: false,
            web: WebUi::Dir(dir.join("web")),
            llama_url: "http://127.0.0.1:1".into(),
            data_dir: dir.clone(),
            engine_note: Arc::new(Mutex::new(None)),
            log: None,
            manage_engine: false,
        })
        .await
        .unwrap();
        let port = running.addr.port();
        let too_long = "ä".repeat(MESSAGE_MAX_CHARS + 1);

        // Over the socket: the longest text the hub takes still fits into a frame, and is refused
        // with a notice for this device; the socket stays open.
        let mut ws = Socket::open(port, &running.token).await;
        ws.send(&json!({ "type": "send", "chat_id": chat, "text": too_long }).to_string()).await;
        assert_eq!(ws.notice().await.as_deref(), Some("too_long"));
        ws.send(&json!({ "type": "send", "chat_id": chat, "text": " " }).to_string()).await;
        assert_eq!(ws.notice().await.as_deref(), Some("empty"), "the same socket goes on");
        // A frame beyond the socket's limit ends the connection before it is read into memory.
        let mut big = Socket::open(port, &running.token).await;
        big.send(&"x".repeat((1 << 20) + 1)).await;
        assert_eq!(big.notice().await, None);

        // Over HTTP: editing a message.
        let edit = |text: String| {
            reqwest::Client::new()
                .put(format!("http://127.0.0.1:{port}/api/messages/{greeting}"))
                .header("authorization", format!("Bearer {}", running.token))
                .json(&json!({ "content": text }))
                .send()
        };
        let res = edit(too_long).await.unwrap();
        assert_eq!(res.status().as_u16(), 400);
        assert_eq!(res.json::<Value>().await.unwrap()["error"], "too_long");
        assert_eq!(edit("Hallo.".into()).await.unwrap().status().as_u16(), 204);
        running.stop();
        let _ = std::fs::remove_dir_all(dir);
    }
}
