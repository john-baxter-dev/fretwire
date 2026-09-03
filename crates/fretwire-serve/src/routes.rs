//! The HTTP surface: embedded static frontend, `POST /invoke/{command}`, and the `/events`
//! WebSocket — plus the two guards every non-static request passes: the Origin/Host check and
//! the single-editor lease.

use crate::ServeSink;
use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::{DefaultBodyLimit, Path, Query, State, WebSocketUpgrade};
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
/// WebSocket close code for a missing or wrong token, mirroring HTTP 401. Accept-then-close like
/// the lease refusal, so the page can ask for the token instead of retrying against a 1006.
const CLOSE_UNAUTHORIZED: u16 = 4401;

pub struct Served {
    pub app: AppState,
    events: broadcast::Sender<String>,
    /// The single-editor lease: the `client` id of the page whose WebSocket is connected.
    /// `AppState`'s clipboards and the session's undo history are single-editor state, so a
    /// second concurrent browser is refused rather than silently sharing them
    /// (`docs/serve-mode.md`, open questions). Released when that socket closes, so a page
    /// refresh reclaims it.
    lease: Mutex<Lease>,
    /// `index.html` with the transport marker injected, prepared once at startup.
    index: String,
    /// The bearer token every invoke and event socket must carry, when the bind is wider than
    /// loopback (or one was given anyway). `None` on a plain loopback bind — see `crate::token`.
    token: Option<String>,
    /// The bound port, which a `Host` header must name when the token stands in for the
    /// loopback rule.
    port: u16,
}

/// How long the daemon may sit with no editor connected before it closes the device session,
/// returning the pedal to standalone. Long enough that a refresh, a Wi-Fi blip, or a short
/// laptop sleep never costs the undo history (the session survives those); short enough that a
/// closed tab doesn't leave the USB interface claimed all night — an unclean host shutdown with
/// a session open is the one state that leaves the pedal needing a power cycle.
const IDLE_CLOSE: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// The lease with a generation counter, bumped on every claim *and* release — so an idle-close
/// timer armed at one release can tell whether anything happened since, and a stale timer never
/// closes a session an editor came back to.
#[derive(Default)]
struct Lease {
    holder: Option<String>,
    generation: u64,
}

impl Lease {
    /// Claim for `client`; false if another client holds it. Re-claiming one's own is fine
    /// (a reconnect after a network blip).
    fn claim(&mut self, client: &str) -> bool {
        match self.holder.as_deref() {
            Some(held) if held != client => false,
            _ => {
                self.holder = Some(client.to_string());
                self.generation += 1;
                true
            }
        }
    }

    /// Release if `client` still holds it, returning the generation to arm an idle timer with.
    fn release(&mut self, client: &str) -> Option<u64> {
        if self.holder.as_deref() != Some(client) {
            return None;
        }
        self.holder = None;
        self.generation += 1;
        Some(self.generation)
    }

    /// Whether nothing has happened since the timer was armed at `generation`.
    fn still_idle(&self, generation: u64) -> bool {
        self.holder.is_none() && self.generation == generation
    }
}

impl Served {
    pub fn new(
        app: AppState,
        events: broadcast::Sender<String>,
        token: Option<String>,
        port: u16,
    ) -> Self {
        let raw = Assets::get("index.html").expect("dist/index.html is embedded (build.rs)");
        let raw = String::from_utf8(raw.data.into_owned()).expect("index.html is UTF-8");
        // The marker must be an inline, non-module script so it runs before Vite's deferred
        // module scripts — ipc.js picks its transport at module evaluation. `auth` tells the
        // page to ask for a token up front when it has none, instead of finding out on the
        // first 401.
        let marker = serde_json::json!({ "auth": token.is_some() });
        let index = raw.replacen(
            "<head>",
            &format!("<head><script>window.__FRETWIRE_SERVE__={marker}</script>"),
            1,
        );
        assert!(
            index.contains("__FRETWIRE_SERVE__"),
            "index.html has no <head> to inject the serve marker into"
        );
        Self {
            app,
            events,
            lease: Mutex::new(Lease::default()),
            index,
            token,
            port,
        }
    }
}

/// The most an invoke body may carry, in bytes. See the `/invoke` route.
const INVOKE_BODY_LIMIT: usize = 64 * 1024 * 1024;

pub fn router(srv: Arc<Served>) -> Router {
    Router::new()
        .route("/", get(index))
        // axum's default cap is 2 MB. The `_inline` file commands carry the file in the body —
        // a restore sends the whole export back (a Floor's eight setlists run past that) and an
        // IR is a few KB — so the cap is raised to a size no honest request approaches.
        .route(
            "/invoke/{command}",
            post(invoke).layer(DefaultBodyLimit::max(INVOKE_BODY_LIMIT)),
        )
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
    if let Err(refused) = origin_guard(&srv, &headers) {
        return refused;
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if !srv.authorized(bearer) {
        return (StatusCode::UNAUTHORIZED, "missing or wrong token").into_response();
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
        if let Some(held) = lease.holder.as_deref()
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
    /// The bearer token, as a query parameter because browser JavaScript cannot set headers on
    /// a WebSocket handshake. Same wire as everything else; the daemon's own logs don't record
    /// query strings.
    token: Option<String>,
}

async fn events(
    State(srv): State<Arc<Served>>,
    Query(q): Query<EventsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(refused) = origin_guard(&srv, &headers) {
        return refused;
    }
    let authorized = srv.authorized(q.token.as_deref());
    ws.on_upgrade(move |socket| event_socket(srv, q.client, authorized, socket))
}

async fn event_socket(srv: Arc<Served>, client: String, authorized: bool, mut socket: WebSocket) {
    if !authorized {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: CLOSE_UNAUTHORIZED,
                reason: "missing or wrong token".into(),
            })))
            .await;
        return;
    }
    let claimed = srv.lease.lock().expect("lease lock").claim(&client);
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

    let armed = srv.lease.lock().expect("lease lock").release(&client);
    if let Some(generation) = armed {
        tracing::info!("editor disconnected");
        spawn_idle_close(srv, generation);
    }
}

/// Arm the idle close: if no editor has held the lease for [`IDLE_CLOSE`] after this release,
/// tear the session down cleanly so the pedal returns to standalone. The generation check under
/// the lease lock makes a stale timer a no-op — any claim since (even a claim-and-release, which
/// armed its own fresher timer) bumps it.
fn spawn_idle_close(srv: Arc<Served>, generation: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(IDLE_CLOSE).await;
        let taken = {
            let lease = srv.lease.lock().expect("lease lock");
            if !lease.still_idle(generation) {
                return;
            }
            // Taken under the lease lock so a claim can't slip in between the check and the
            // take. (Invokes lock lease then session sequentially, never nested — no deadlock.)
            srv.app.session.lock().ok().and_then(|mut g| g.take())
        };
        if let Some(mut session) = taken {
            tracing::info!(
                "no editor for {} min — closing the device session; the pedal is standalone again",
                IDLE_CLOSE.as_secs() / 60
            );
            let _ = tokio::task::spawn_blocking(move || session.close()).await;
        }
    });
}

/// The Origin/Host check, on for every non-static request regardless of bind address: a local
/// HTTP server without one is reachable by any web page the browser visits, via DNS rebinding,
/// firewalls notwithstanding (`docs/serve-mode.md` §4).
fn origin_guard(srv: &Served, headers: &HeaderMap) -> Result<(), Response> {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let tokened = srv.token.is_some();
    if !host.is_some_and(|h| host_allowed(h, tokened, srv.port)) {
        return Err((StatusCode::FORBIDDEN, "refused: unexpected Host").into_response());
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let authority = origin.split_once("://").map(|(_, rest)| rest);
        // With a token the page may be at any name (an IP, `pi.local`) — the rule is same-origin
        // with the Host actually used. Without one, the loopback rule stands on both.
        let ok = if tokened {
            authority == host
        } else {
            authority.is_some_and(host_is_loopback)
        };
        if !ok {
            return Err((StatusCode::FORBIDDEN, "refused: cross-origin request").into_response());
        }
    }
    Ok(())
}

impl Served {
    /// Whether `given` unlocks this daemon: always, with no token configured; otherwise only an
    /// exact (constant-time) match.
    fn authorized(&self, given: Option<&str>) -> bool {
        match &self.token {
            None => true,
            Some(expected) => given.is_some_and(|g| crate::token::matches(expected, g)),
        }
    }
}

/// Whether a `Host` header value may reach the API. Loopback always. With a token configured,
/// any name on our port: DNS rebinding can make a hostile page land here, but it then runs on
/// its own origin, with no token, and gets a 401 — the token is the defense, and the daemon
/// cannot know which names (an IP, `pi.local`, a DNS entry) the user legitimately types.
fn host_allowed(value: &str, tokened: bool, port: u16) -> bool {
    host_is_loopback(value) || (tokened && host_port(value) == Some(port))
}

/// The explicit port in a `Host`-style value, if any (`[::1]:8317`, `pi.local:8317`).
fn host_port(value: &str) -> Option<u16> {
    let after_host = match value.strip_prefix('[') {
        Some(rest) => rest.split_once(']').map(|(_, tail)| tail)?,
        None => value.rsplit_once(':').map(|(_, port)| port)?,
    };
    after_host
        .strip_prefix(':')
        .unwrap_or(after_host)
        .parse()
        .ok()
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
    use super::{Lease, host_allowed, host_is_loopback, host_port};

    /// The idle-close arming rules: a stale timer must see any claim since its release — even a
    /// claim-and-release pair, which armed its own fresher timer.
    #[test]
    fn a_stale_idle_timer_is_a_no_op() {
        let mut lease = Lease::default();
        assert!(lease.claim("a"));
        assert!(!lease.claim("b"), "second editor refused");
        assert!(
            lease.claim("a"),
            "reclaiming one's own is a reconnect, not a conflict"
        );
        assert_eq!(lease.release("b"), None, "only the holder releases");
        let armed = lease.release("a").expect("holder releases");
        assert!(
            lease.still_idle(armed),
            "nothing happened yet — timer may fire"
        );
        assert!(lease.claim("b"), "released lease is claimable");
        assert!(!lease.still_idle(armed), "a claim disarms the old timer");
        let rearmed = lease.release("b").expect("holder releases");
        assert!(
            !lease.still_idle(armed),
            "the fresher timer owns this idle span"
        );
        assert!(lease.still_idle(rearmed));
    }

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

    /// The token relaxes the Host rule to "our port" and nothing else: loopback still passes,
    /// a foreign name on our port passes (the token, not the name, is the defense), any other
    /// port does not, and without a token the loopback rule is untouched.
    #[test]
    fn a_token_admits_any_name_on_our_port() {
        assert!(host_allowed("localhost:8317", true, 8317));
        assert!(host_allowed("pi.local:8317", true, 8317));
        assert!(host_allowed("192.168.1.20:8317", true, 8317));
        assert!(host_allowed("[fe80::1]:8317", true, 8317));
        assert!(!host_allowed("pi.local:8318", true, 8317));
        assert!(
            !host_allowed("pi.local", true, 8317),
            "no port is not our port"
        );
        assert!(!host_allowed("pi.local:8317", false, 8317));
        assert!(host_allowed("127.0.0.1:8317", false, 8317));
    }

    #[test]
    fn host_ports_parse() {
        assert_eq!(host_port("pi.local:8317"), Some(8317));
        assert_eq!(host_port("[::1]:8317"), Some(8317));
        assert_eq!(host_port("[::1]"), None);
        assert_eq!(host_port("pi.local"), None);
        assert_eq!(host_port("pi.local:x"), None);
    }
}
