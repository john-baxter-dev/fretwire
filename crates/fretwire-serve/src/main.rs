//! fretwire-serve — the editor over HTTP, for headless machines (`docs/serve-mode.md`).
//!
//! Serves the same built frontend the Tauri GUI embeds, answers its `invoke()` calls on
//! `POST /invoke/<command>` via `fretwire_commands::dispatch`, and pushes the three backend
//! events (`device-pushes` / `device-lost` / `backup-progress`) over a WebSocket at `/events`.
//! Run it on the machine the pedal is plugged into; open it in a browser.
//!
//! **Loopback only, deliberately.** This is write access to someone's rig, so until the token
//! flow exists (`docs/serve-mode.md` §4) the bind address must be loopback and a non-loopback
//! `--bind` is refused. Reach it from another machine through an SSH tunnel. The Origin/Host
//! check in `routes` is on regardless — DNS rebinding reaches a loopback server from any web
//! page the browser visits.

mod routes;

use clap::Parser;
use fretwire_commands::AppState;
use fretwire_commands::events::{Event, EventSink};
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Parser)]
#[command(name = "fretwire-serve", version, about)]
struct Args {
    /// Address to bind. Loopback only for now; from another machine, tunnel:
    /// `ssh -L 8317:127.0.0.1:8317 <host>`.
    #[arg(long, default_value = "127.0.0.1:8317")]
    bind: std::net::SocketAddr,
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

    if !args.bind.ip().is_loopback() {
        eprintln!(
            "fretwire-serve binds loopback only for now — this is write access to your rig, and \
             the token flow for wider binds isn't built yet (docs/serve-mode.md §4).\n\
             From another machine, tunnel instead:  ssh -L 8317:127.0.0.1:{} <this-host>",
            args.bind.port()
        );
        std::process::exit(2);
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the tokio runtime")
        .block_on(serve(args));
}

async fn serve(args: Args) {
    let (events, _) = broadcast::channel(64);
    let srv = Arc::new(routes::Served::new(AppState::default(), events.clone()));

    // Same keepalive as the GUI's setup hook — the pedal needs its status channel drained
    // whether the frontend is a webview or a browser across the room.
    fretwire_commands::spawn_heartbeat(ServeSink(events), srv.app.session.clone());

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .unwrap_or_else(|e| panic!("binding {}: {e}", args.bind));
    tracing::info!("serving the editor on http://{}/", args.bind);

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
