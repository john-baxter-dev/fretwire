//! fretwire-mcp — the editor as an MCP server, for AI assistants (Claude Code, Claude Desktop,
//! anything that speaks the Model Context Protocol over stdio).
//!
//! Not a mechanical bridge over the 70-command surface: a **curated dozen-odd tools** with
//! summarized, human-readable results (`server.rs`), because the command layer's DTOs exist to
//! *render*, not to reason over. Half of them need no pedal — they read fretwire export files and
//! the model catalog — and those are the half most of the interesting work lives in.
//!
//! **Safety is gated, not defaulted.** Out of the box the server is read-only: it can describe,
//! list, diff and export, and connect to the pedal to read. Tools that change the edit buffer
//! exist only with `--allow-writes`; the one that writes flash (`preset_save`) only with
//! `--allow-save`. An ungated tool is simply not listed, so an assistant cannot be talked into a
//! write the operator didn't enable. Firmware, flash and DFU traffic never appear here, per
//! `docs/safety.md`.
//!
//! stdout is the protocol channel — every log line goes to stderr.

mod offline;
mod server;
mod summary;

use clap::Parser;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[derive(Parser)]
#[command(name = "fretwire-mcp", version, about)]
struct Args {
    /// Expose the tools that change the pedal's edit buffer (parameters, bypass, blocks,
    /// snapshots, preset changes, undo). Nothing persists until a save; a power cycle reverts.
    #[arg(long)]
    allow_writes: bool,
    /// Expose `preset_save`, which overwrites a preset in flash. Implies --allow-writes.
    #[arg(long)]
    allow_save: bool,
}

fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .with_writer(std::io::stderr)
        .init();
    tracing::info!(
        version = fretwire_core::VERSION,
        commit = fretwire_core::BUILD_ID,
        writes = args.allow_writes || args.allow_save,
        save = args.allow_save,
        "fretwire-mcp starting"
    );
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("building the tokio runtime")
        .block_on(run(args));
}

async fn run(args: Args) {
    let gates = server::Gates {
        writes: args.allow_writes || args.allow_save,
        save: args.allow_save,
    };
    let fretwire = server::Fretwire::new(gates);
    let session = fretwire.session();

    // Same keepalive as the GUI and the daemon — a long-lived process holding a session must
    // drain the pedal's status channel. Pushes are logged and otherwise dropped: the assistant
    // reads fresh state on every call.
    fretwire_commands::spawn_heartbeat(server::LogSink, session.clone());

    let running = match fretwire.serve(stdio()).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("MCP handshake failed: {e}");
            std::process::exit(1);
        }
    };
    let quit = running.waiting().await;
    tracing::info!(?quit, "client went away");

    // Mirror the GUI's exit handler: tear the session down cleanly so the pedal returns to
    // standalone, exactly as HX Edit does when it quits.
    let taken = session.lock().ok().and_then(|mut g| g.take());
    if let Some(mut s) = taken {
        tracing::info!("closing the device session");
        let _ = s.close();
    }
}

/// `RUST_LOG` with nusb damped to warnings unless asked for — its debug output is a firehose.
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
