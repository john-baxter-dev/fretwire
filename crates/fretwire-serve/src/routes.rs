//! The HTTP surface: embedded static frontend, `POST /invoke/{command}`, and the `/events`
//! WebSocket — plus the two guards every non-static request passes: the Origin/Host check and
//! the single-editor lease.

use crate::ServeSink;
use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use fretwire_commands::AppState;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// The built frontend, shared with fretwire-tauri. Embedded in release; read from disk in debug.
#[derive(rust_embed::RustEmbed)]
#[folder = "../fretwire-tauri/dist/"]
struct Assets;

/// WebSocket close code for "another editor holds the lease" — in the 4000–4999 private-use
/// range, mirroring HTTP 409. The client must not reconnect on it.
const CLOSE_CONFLICT: u16 = 4409;

pub struct Served {
    pub app: AppState,
    events: broadcast::Sender<String>,
    /// The single-editor lease: the `client` id of the page whose WebSocket is connected.
    /// `AppState`'s clipboards and the session's undo history are single-editor state, so a
    /// second concurrent browser is refused rather than silently sharing them
    /// (`docs/serve-mode.md`, open questions). Released when that socket closes, so a page
    /// refresh reclaims it.
    lease: Mutex<Option<String>>,
    /// `index.html` with the transport marker injected, prepared once at startup.
    index: String,
}

impl Served {
    pub fn new(app: AppState, events: broadcast::Sender<String>) -> Self {
        let raw = Assets::get("index.html").expect("dist/index.html is embedded (build.rs)");
        let raw = String::from_utf8(raw.data.into_owned()).expect("index.html is UTF-8");
        // The marker must be an inline, non-module script so it runs before Vite's deferred
        // module scripts — ipc.js picks its transport at module evaluation.
        let index = raw.replacen(
            "<head>",
            "<head><script>window.__FRETWIRE_SERVE__={}</script>",
            1,
        );
        assert!(
            index.contains("__FRETWIRE_SERVE__"),
            "index.html has no <head> to inject the serve marker into"
        );
        Self {
            app,
            events,
            lease: Mutex::new(None),
            index,
        }
    }
}

pub fn router(srv: Arc<Served>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/invoke/{command}", post(invoke))
        .route("/events", get(events))
        .route("/{*path}", get(asset))
        .with_state(srv)
}

async fn index(State(srv): State<Arc<Served>>) -> Html<String> {
    Html(srv.index.clone())
}

async fn asset(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(f) => (
            [(header::CONTENT_TYPE, content_type(&path))],
            f.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// By extension — Vite's output is a handful of known types.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css",
        Some("js" | "mjs") => "text/javascript",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("json" | "map") => "application/json",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

async fn invoke(
    State(srv): State<Arc<Served>>,
    Path(command): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Err(refused) = origin_guard(&headers) {
        return refused;
    }
    // Forms can't send application/json, so requiring it shuts out CSRF POSTs for free. (The
    // body is taken as a String and parsed by hand because axum's Json extractor would answer
    // with its own error shape; the frontend expects plain strings.)
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if !is_json {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invoke requires Content-Type: application/json",
        )
            .into_response();
    }
    let args: Value = match serde_json::from_str(if body.is_empty() { "{}" } else { &body }) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")).into_response(),
    };

    // The lease: while a WebSocket holds it, invokes from any other page are refused.
    let client = headers
        .get("x-fretwire-client")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    {
        let lease = srv.lease.lock().expect("lease lock");
        if let Some(held) = lease.as_deref()
            && held != client
        {
            return (StatusCode::CONFLICT, "another editor is connected").into_response();
        }
    }

    let sink = ServeSink(srv.events.clone());
    match fretwire_commands::dispatch::dispatch(&srv.app, sink, &command, args).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct EventsQuery {
    /// The page's random id — the lease key.
    client: String,
}

async fn events(
    State(srv): State<Arc<Served>>,
    Query(q): Query<EventsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(refused) = origin_guard(&headers) {
        return refused;
    }
    ws.on_upgrade(move |socket| event_socket(srv, q.client, socket))
}

async fn event_socket(srv: Arc<Served>, client: String, mut socket: WebSocket) {
    let claimed = {
        let mut lease = srv.lease.lock().expect("lease lock");
        match lease.as_deref() {
            Some(held) if held != client => false,
            _ => {
                *lease = Some(client.clone());
                true
            }
        }
    };
    if !claimed {
        // Accept-then-close so the browser sees the code; a refused upgrade would be
        // indistinguishable from the server being down.
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: CLOSE_CONFLICT,
                reason: "another editor is connected".into(),
            })))
            .await;
        return;
    }
    tracing::info!("editor connected");

    let mut rx = srv.events.subscribe();
    loop {
        tokio::select! {
            frame = rx.recv() => match frame {
                Ok(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // A slow client skips a burst rather than killing the connection — pushes are
                // advisory live-follow, and every mutation returns fresh state anyway.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(_)) => continue, // pings and the like; the client never sends data
                _ => break,              // closed or errored — the page went away
            },
        }
    }

    let mut lease = srv.lease.lock().expect("lease lock");
    if lease.as_deref() == Some(client.as_str()) {
        *lease = None;
        tracing::info!("editor disconnected");
    }
}

/// The Origin/Host check, on for every non-static request regardless of bind address: a local
/// HTTP server without one is reachable by any web page the browser visits, via DNS rebinding,
/// firewalls notwithstanding (`docs/serve-mode.md` §4).
fn origin_guard(headers: &HeaderMap) -> Result<(), Response> {
    let host_ok = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(host_is_loopback);
    if !host_ok {
        return Err((StatusCode::FORBIDDEN, "refused: unexpected Host").into_response());
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let ok = origin
            .split_once("://")
            .map(|(_, rest)| rest)
            .is_some_and(host_is_loopback);
        if !ok {
            return Err((StatusCode::FORBIDDEN, "refused: cross-origin request").into_response());
        }
    }
    Ok(())
}

/// Whether a `Host`-style value (name or address, optional port) is loopback.
fn host_is_loopback(value: &str) -> bool {
    let host = match value.strip_prefix('[') {
        // Bracketed IPv6: `[::1]:8317`.
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => value.rsplit_once(':').map_or(value, |(h, _)| h),
    };
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::host_is_loopback;

    #[test]
    fn loopback_hosts_pass() {
        for h in [
            "localhost",
            "localhost:8317",
            "127.0.0.1",
            "127.0.0.1:8317",
            "[::1]:8317",
        ] {
            assert!(host_is_loopback(h), "{h} should pass");
        }
    }

    /// The DNS-rebinding shape: a name that isn't ours, resolving wherever the attacker likes —
    /// the check is on the *name*, which the attacker cannot fake in the browser.
    #[test]
    fn foreign_hosts_are_refused() {
        for h in [
            "evil.example",
            "evil.example:8317",
            "192.168.1.20:8317",
            "fretwire.localhost.evil.example",
            "",
        ] {
            assert!(!host_is_loopback(h), "{h} should be refused");
        }
    }
}
