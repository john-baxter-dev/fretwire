//! fretwire-serve — the editor over HTTP, for headless machines (`docs/serve-mode.md`).
//!
//! Serves the same built frontend the Tauri GUI embeds, answers its `invoke()` calls on
//! `POST /invoke/<command>` via `fretwire_commands::dispatch`, and pushes the three backend
//! events (`device-pushes` / `device-lost` / `backup-progress`) over a WebSocket at `/events`.
//! Run it on the machine the pedal is plugged into; open it in a browser.
//!
//! **Loopback by default; a token anywhere wider.** This is write access to someone's rig. On
//! loopback only local processes reach the port and nothing more is asked. Any other bind
//! requires a bearer token (`token`), generated once and printed at startup inside the link to
//! open; every invoke and the event socket must carry it. Traffic is plain HTTP, so a wider bind
//! assumes a trusted network — from anywhere else, tunnel (`ssh -L 8317:127.0.0.1:8317 <host>`)
//! or put a VPN or a TLS proxy in front (`docs/serve-mode.md` §4). The Origin/Host check in
//! `routes` is on regardless — DNS rebinding reaches a loopback server from any web page the
//! browser visits.

mod routes;
mod token;

use clap::Parser;
use fretwire_commands::AppState;
use fretwire_commands::events::{Event, EventSink};
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Parser)]
#[command(name = "fretwire-serve", version, about)]
struct Args {
    /// Address to bind. Loopback needs no token. Any other address requires one (generated and
    /// kept on first use, printed at startup as the link to open) and is plain HTTP, so it
    /// assumes a trusted network — from anywhere else, tunnel: `ssh -L 8317:127.0.0.1:8317 <host>`.
    #[arg(long, default_value = "127.0.0.1:8317")]
    bind: std::net::SocketAddr,
    /// Use this token instead of the stored one. On loopback, where none is needed, giving one
    /// demands it there too.
    #[arg(long, env = "FRETWIRE_SERVE_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Where the generated token is kept (mode 0600). Default: `serve-token` next to the data
    /// directory, i.e. `~/.local/share/fretwire/serve-token`.
    #[arg(long)]
    token_file: Option<std::path::PathBuf>,
}

/// [`EventSink`] over the broadcast channel every connected WebSocket subscribes to. The frame is
/// serialized once, here, so all subscribers see identical bytes. No receivers (no browser open)
/// is not an error — events while nobody watches are simply dropped, same as Tauri emitting to a
/// closed window.
#[derive(Clone)]
pub struct ServeSink(pub broadcast::Sender<String>);

impl EventSink for ServeSink {
    fn emit(&self, event: Event) {
        let frame = serde_json::json!({
            "event": event.name(),
            "payload": event.payload(),
        });
        let _ = self.0.send(frame.to_string());
    }
}

fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .init();
    tracing::info!(
        version = fretwire_core::VERSION,
        commit = fretwire_core::BUILD_ID,
        "fretwire-serve starting"
    );

    let token = match (&args.token, args.bind.ip().is_loopback()) {
        (Some(t), _) => Some(t.trim().to_string()).filter(|t| !t.is_empty()),
        (None, true) => None,
        (None, false) => {
            let path = args
                .token_file
                .clone()
                .unwrap_or_else(|| token_path_default());
            match token::load_or_create(&path) {
                Ok((t, created)) => {
                    if created {
                        tracing::info!(path = %path.display(), "generated a new token");
                    } else {
                        tracing::info!(path = %path.display(), "using the stored token");
                    }
                    Some(t)
                }
                Err(e) => {
                    eprintln!(
                        "fretwire-serve: a bind beyond loopback needs a token, and reading or \
                         creating {} failed: {e}\nPass one with --token, or bind loopback and \
                         tunnel:  ssh -L 8317:127.0.0.1:{} <this-host>",
                        path.display(),
                        args.bind.port()
                    );
                    std::process::exit(2);
                }
            }
        }
    };
    if token.is_none() && !args.bind.ip().is_loopback() {
        eprintln!("fretwire-serve: --token must not be empty for a bind beyond loopback");
        std::process::exit(2);
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the tokio runtime")
        .block_on(serve(args, token));
}

/// `~/.local/share/fretwire/serve-token` — beside the data dir, not inside it, so an import or
/// a wipe of the reference data never touches it.
fn token_path_default() -> std::path::PathBuf {
    let data = fretwire_core::data_dir();
    data.parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(data)
        .join("serve-token")
}

async fn serve(args: Args, token: Option<String>) {
    let (events, _) = broadcast::channel(64);
    let srv = Arc::new(routes::Served::new(
        AppState::default(),
        events.clone(),
        token.clone(),
        args.bind.port(),
    ));

    // Same keepalive as the GUI's setup hook — the pedal needs its status channel drained
    // whether the frontend is a webview or a browser across the room.
    fretwire_commands::spawn_heartbeat(ServeSink(events), srv.app.session.clone());

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .unwrap_or_else(|e| panic!("binding {}: {e}", args.bind));
    match &token {
        // The link is the credential: the fragment carries the token to the page, which keeps it
        // and drops it from the address bar. Printed rather than logged-only so it is the first
        // thing a headless setup shows.
        Some(t) => {
            let host = if args.bind.ip().is_unspecified() {
                "<this machine's address>".to_string()
            } else {
                match args.bind.ip() {
                    std::net::IpAddr::V6(v6) => format!("[{v6}]"),
                    v4 => v4.to_string(),
                }
            };
            tracing::info!(
                "serving the editor on http://{}/ (token required)",
                args.bind
            );
            eprintln!(
                "\nOpen the editor at:\n\n    http://{host}:{}/#token={t}\n\nThe link carries \
                 the token; anyone with it can edit the pedal. Plain HTTP — use it on a network \
                 you trust.\n",
                args.bind.port()
            );
        }
        None => tracing::info!("serving the editor on http://{}/", args.bind),
    }

    axum::serve(listener, routes::router(srv.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serving");

    // Mirror the GUI's exit handler: tear the session down cleanly so the pedal returns to
    // standalone — no "panel lock" — exactly as HX Edit does when it quits.
    let taken = srv.app.session.lock().ok().and_then(|mut g| g.take());
    if let Some(mut session) = taken {
        tracing::info!("closing the device session");
        let _ = session.close();
    }
}

/// Resolves on Ctrl-C or SIGTERM — both must release the USB interface cleanly.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("installing the SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {},
        _ = term.recv() => {},
    }
}

/// The log filter, with `nusb` damped unless asked for by name — same reasoning as the CLI's and
/// the GUI's copy (each binary owns its own): `RUST_LOG=debug` would otherwise be 94% per-URB
/// noise.
fn log_filter() -> tracing_subscriber::EnvFilter {
    match std::env::var("RUST_LOG") {
        Ok(v) if !v.is_empty() => {
            let damped = if v.contains("nusb") {
                v
            } else {
                format!("{v},nusb=warn")
            };
            tracing_subscriber::EnvFilter::new(damped)
        }
        _ => tracing_subscriber::EnvFilter::new("info,nusb=warn"),
    }
}
