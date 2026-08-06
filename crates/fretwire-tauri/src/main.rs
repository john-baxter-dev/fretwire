// Prevents a second console window on Windows (harmless on Linux; kept for parity).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! fretwire — Tauri (WebKitGTK webview) front end over the `fretwire-core`
//! stack. The Rust core (protocol/transport/preset decode/`Session`/`Catalog`) is reused unchanged;
//! this crate adds the `#[command]` surface (see [`commands`]) and the serde wire types ([`dto`]).
//! The live session lives in Tauri-managed [`commands::AppState`]. Run: `cargo run -p fretwire-tauri`.

mod commands;
mod dto;

use tauri::Manager;

fn main() {
    // Logs go to the terminal; run with `RUST_LOG=trace cargo run -p fretwire-tauri` to trace USB I/O.
    tracing_subscriber::fmt()
        .with_env_filter(fretwire_log_filter())
        .init();

    // WebKitGTK's default dmabuf renderer hits a fatal Wayland protocol error on this GPU/compositor
    // (the same dmabuf/EGL path that ruled out wgpu for iced). Force WebKitGTK's non-dmabuf fallback
    // compositing before GTK initializes — the escape hatch the GPU backend never offered. Honor an
    // explicit override if the user already set it.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY (Rust 2024 made `set_var` unsafe: mutating the environment races other threads
        // reading it): this runs at the very top of `main`, before `tauri::Builder` — and thus
        // before GTK/WebKit or the async runtime spawn any thread — so the process is still
        // single-threaded and no concurrent env access is possible.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .setup(|app| {
            // Keepalive heartbeat: drains the device's status channel while connected so queued
            // status-pushes don't desync the next read (see commands::spawn_heartbeat).
            let session = app.state::<commands::AppState>().session.clone();
            commands::spawn_heartbeat(app.handle().clone(), session);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::data_status,
            commands::import_data,
            commands::detect,
            commands::is_connected,
            commands::connect,
            commands::disconnect,
            commands::read_preset,
            commands::undo,
            commands::redo,
            commands::history_jump,
            commands::preview_param,
            commands::preview_paired_param,
            commands::set_bypass,
            commands::set_param,
            commands::set_paired_param,
            commands::set_param_enum,
            commands::swap_model,
            commands::add_block,
            commands::add_block_at,
            commands::delete_block,
            commands::reorder_block,
            commands::move_block_to_row,
            commands::move_before_split,
            commands::place_block,
            commands::insert_block,
            commands::set_node_pos,
            commands::set_split_type,
            commands::set_snapshot,
            commands::goto_preset,
            commands::save_preset,
            commands::rename_preset,
            commands::rename_snapshot,
            commands::list_presets,
            commands::setlists,
            commands::cross_setlist_write_allowed,
            commands::backup_setlist,
            commands::backup_show,
            commands::restore_preset,
            commands::split_types,
            commands::categories,
            commands::models_in_category,
            commands::copy_preset,
            commands::paste_preset,
            commands::clipboard_preset,
            commands::copy_block,
            commands::paste_block,
            commands::clipboard_block,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // On app exit (window closed), tear the session down cleanly so the pedal returns to
            // standalone — no "panel lock" — exactly as HX Edit does when it quits.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app_handle.state::<commands::AppState>();
                let taken = state.session.lock().ok().and_then(|mut g| g.take());
                if let Some(mut session) = taken {
                    let _ = session.close();
                }
            }
        });
}

/// The log filter, with `nusb` damped unless the user asked for it by name.
///
/// `RUST_LOG=debug` turns on nusb's per-URB tracing, which is **94% of a bug-report log** by volume
/// (7.2 MB of a Floor session's 7.7 MB) and buries the protocol lines a report is actually about.
/// An explicit `nusb=…` directive still wins, so `RUST_LOG=debug,nusb=debug` gets the URBs back.
fn fretwire_log_filter() -> tracing_subscriber::EnvFilter {
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
