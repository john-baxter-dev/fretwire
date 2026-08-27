// Prevents a second console window on Windows (harmless on Linux; kept for parity).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! fretwire — Tauri (WebKitGTK webview) front end over the `fretwire-core`
//! stack. The Rust core (protocol/transport/preset decode/`Session`/`Catalog`) is reused unchanged;
//! this crate adds the `#[command]` surface (see [`commands`]) and the serde wire types ([`dto`]).
//! The live session lives in Tauri-managed [`commands::AppState`]. Run: `cargo run -p fretwire-tauri`.

mod commands;
mod dto;

use tauri::Manager;

/// Shrink the configured window size to fit the screen it opens on, then re-centre.
///
/// `tauri.conf.json` asks for 1360x860, which is what it takes to see a full HX Stomp chain without
/// scrolling — the chain draws `84 + columns * 120` px (`Chain.svelte`), so eight occupied columns
/// is 1044, and the sidebar, page padding and workspace gap add 278 around it. 1280 left it 42px
/// short, which showed up as having to scroll to reach the Out node.
///
/// That is wider than a 1366x768 laptop is tall, though, and Tauri takes the configured size
/// literally: it would open with the bottom of the window (the param panel) off the screen, on
/// exactly the machines least able to spare the room.
///
/// Only ever shrinks. Growing to fill a large monitor would be its own kind of rude, and the
/// minimums in the config still apply — on a screen smaller than those, the window is oversized
/// and scrollable rather than clipped to something unusable.
///
/// Best-effort throughout: a compositor that reports no monitor (some headless and remote-display
/// setups) leaves the configured size alone, which is the pre-2026-08-22 behaviour.
fn fit_window_to_screen(app: &tauri::App) {
    use tauri::{LogicalSize, Manager};

    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let Ok(Some(monitor)) = win.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    // The work area excludes panels and docks, so this is the space actually available. Both the
    // measurement and the write below are *inner* sizes (`set_size` sets the content area), so the
    // margins here are what covers the decorations the work area doesn't know about.
    let area = monitor.work_area();
    let avail_w = f64::from(area.size.width) / scale - 24.0;
    let avail_h = f64::from(area.size.height) / scale - 64.0;

    let Ok(size) = win.inner_size() else {
        return;
    };
    let want = (
        f64::from(size.width) / scale,
        f64::from(size.height) / scale,
    );
    let Some((w, h)) = shrink_to_fit(want, (avail_w, avail_h)) else {
        return; // it already fits
    };
    let (want_w, want_h) = want;

    if let Err(e) = win.set_size(LogicalSize::new(w, h)) {
        tracing::debug!(error = %e, "could not fit the window to the screen; leaving it as configured");
        return;
    }
    // Resizing from the top-left corner leaves it off-centre, and the config's `center` already ran.
    let _ = win.center();
    tracing::debug!(
        requested = format!("{want_w:.0}x{want_h:.0}"),
        opened = format!("{w:.0}x{h:.0}"),
        "window shrunk to fit the screen"
    );
}

/// The size arithmetic on its own: `Some` new size when `want` doesn't fit in `avail`, `None` when
/// it already does. Split out because the window half of this can't be exercised without a
/// compositor, and the part that can actually be wrong is here.
///
/// Each axis is clamped independently — the common case is a screen wide enough but not tall
/// enough (1366x768, 1920x1080 with a tall panel), where narrowing the window as well would give
/// away width there was no need to.
fn shrink_to_fit(want: (f64, f64), avail: (f64, f64)) -> Option<(f64, f64)> {
    // A monitor that reports nonsense is not a reason to resize to nonsense.
    if !avail.0.is_finite() || !avail.1.is_finite() || avail.0 <= 0.0 || avail.1 <= 0.0 {
        return None;
    }
    let w = want.0.min(avail.0);
    let h = want.1.min(avail.1);
    (w < want.0 || h < want.1).then_some((w.max(1.0), h.max(1.0)))
}

fn main() {
    // Logs go to the terminal; run with `RUST_LOG=trace cargo run -p fretwire-tauri` to trace USB I/O.
    tracing_subscriber::fmt()
        .with_env_filter(fretwire_log_filter())
        .init();
    // First line of every log, so a pasted log says which build produced it.
    tracing::info!(
        version = fretwire_core::VERSION,
        commit = fretwire_core::BUILD_ID,
        "fretwire-gui starting"
    );

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
            fit_window_to_screen(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::data_status,
            commands::device_numbering,
            commands::settings_read,
            commands::settings_write,
            commands::ir_list,
            commands::ir_scan,
            commands::ir_export,
            commands::ir_upload,
            commands::ir_delete,
            commands::ir_rename,
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
            commands::clear_preset,
            commands::reorder_block,
            commands::move_block_to_row,
            commands::move_before_split,
            commands::place_block,
            commands::insert_block,
            commands::set_node_pos,
            commands::set_split_type,
            commands::assign_bypass,
            commands::unassign_bypass,
            commands::assign_param,
            commands::set_assign_travel,
            commands::set_snapshot,
            commands::goto_preset,
            commands::save_preset,
            commands::rename_preset,
            commands::rename_snapshot,
            commands::list_presets,
            commands::setlists,
            commands::cross_setlist_write_allowed,
            commands::export_setlists,
            commands::cancel_export,
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

#[cfg(test)]
mod window_fit_tests {
    use super::shrink_to_fit;

    /// The size the config asks for, in logical pixels.
    const WANT: (f64, f64) = (1360.0, 860.0);

    #[test]
    fn a_roomy_screen_is_left_alone() {
        // 1920x1080 minus the margins: both axes fit, so the window opens as configured.
        assert_eq!(shrink_to_fit(WANT, (1896.0, 1016.0)), None);
        // Exactly the requested size still counts as fitting.
        assert_eq!(shrink_to_fit(WANT, WANT), None);
    }

    /// The axes clamp independently, so a screen with room to spare on one keeps it. This is the
    /// common shape — plenty of width, not quite the height (a 1440x900 panel, or a 1080p screen
    /// under a tall dock).
    #[test]
    fn a_screen_short_on_one_axis_keeps_the_other() {
        let (w, h) = shrink_to_fit(WANT, (1416.0, 836.0)).expect("must shrink");
        assert_eq!(w, 1360.0, "the width fits, so it is kept in full");
        assert_eq!(h, 836.0);
    }

    #[test]
    fn a_smaller_screen_loses_both() {
        // The 1366x768 laptop the clamp exists for: narrower *and* shorter than the window wants
        // now, so both axes give. The chain scrolls there, which is what a 1366px screen buys you.
        assert_eq!(shrink_to_fit(WANT, (1342.0, 704.0)), Some((1342.0, 704.0)));
        assert_eq!(shrink_to_fit(WANT, (1000.0, 700.0)), Some((1000.0, 700.0)));
    }

    #[test]
    fn it_never_grows() {
        // A 4K monitor does not get a 4K window.
        assert_eq!(shrink_to_fit(WANT, (3816.0, 2096.0)), None);
    }

    #[test]
    fn a_nonsense_work_area_changes_nothing() {
        // Better the configured size than a 1x1 window, if a compositor reports zero or NaN.
        assert_eq!(shrink_to_fit(WANT, (0.0, 0.0)), None);
        assert_eq!(shrink_to_fit(WANT, (f64::NAN, 1000.0)), None);
        assert_eq!(shrink_to_fit(WANT, (-100.0, 900.0)), None);
    }
}
