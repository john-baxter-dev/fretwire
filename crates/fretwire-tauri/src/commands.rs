//! The Tauri command surface — thin wrappers over `fretwire_core::Session`, exposing the editor to the
//! Svelte frontend. The live session is held in Tauri-managed state behind an `Arc<Mutex<…>>` (the
//! same pattern the iced GUI uses), so `connect` opens it once and every later command reuses it.
//!
//! **Threading:** all device I/O is blocking USB, so these commands are `async` and run the work on
//! a background thread via `spawn_blocking`. Running them synchronously would block Tauri's main
//! (webview) thread — freezing the UI and deadlocking the very call that's meant to return.
//!
//! Mutating commands re-read the preset and return the fresh `PresetDto`, so the frontend always
//! renders authoritative device state.

use crate::dto::{
    CategoryDto, DataStatusDto, DetectedDeviceDto, ImportResultDto, IrSlotDto, ModelChoiceDto,
    PresetDto, PresetListItem, SettingDto, SplitTypeDto,
};
use fretwire_core::{EditorPreset, Session};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

#[derive(Default)]
pub struct AppState {
    pub session: Arc<Mutex<Option<Session>>>,
    /// Copy/paste buffer for whole presets: the raw stream as read off the device, plus the name it
    /// had. Kept here rather than handed to the frontend because it is ~7 KB of binary that the UI
    /// has no use for — the JS side only ever sees the name, for the button's label.
    pub clipboard: Arc<Mutex<Option<PresetClip>>>,
    /// Copy/paste buffer for a single block.
    pub block_clipboard: Arc<Mutex<Option<BlockClip>>>,
    /// Set by `cancel_export` to call off an export sweep in flight. It lives outside the session
    /// lock on purpose: the sweep holds that lock for its whole run (forty minutes for a Floor's
    /// eight setlists), so a cancel that needed the lock could never arrive.
    pub cancel_export: Arc<AtomicBool>,
}

/// A copied preset: the raw stream exactly as read off the device, and the name it had.
#[derive(Clone)]
pub struct PresetClip {
    pub raw: Vec<u8>,
    pub name: String,
}

/// A copied block: enough to rebuild it in another slot without touching the preset blob.
#[derive(Clone)]
pub struct BlockClip {
    pub name: String,
    pub model_index: i64,
    /// The paired cab/IR for an amp+cab block; `-1` when there isn't one, matching `swap_model`.
    pub paired_index: i64,
    pub bypassed: bool,
    /// `(param index, value)` for the main model and, separately, the paired sub-model. Captured as
    /// typed values so each one goes back on the wire as the type it came off as.
    pub params: Vec<(i64, fretwire_core::fretwire_data::stream::ParamValue)>,
    pub paired_params: Vec<(i64, fretwire_core::fretwire_data::stream::ParamValue)>,
}

/// Spawn the keepalive heartbeat. While a session is open, this beats every 250 ms — same cadence as
/// the iced GUI — sending an idle frame on each channel and draining the device's queued
/// status-pushes/meters. Without it, those pile up between commands and desync the next read (a model
/// swap then times out and every later command hangs on the lock). Runs for the app's lifetime;
/// no-ops while disconnected.
///
/// Device-originated changes (footswitch bypass, panel snapshot/preset switch) are forwarded to the
/// frontend as a `device-pushes` event so the GUI follows the hardware live.
///
/// If the pedal stops answering, the beat **gives up** rather than retrying forever: a wedged device
/// makes every send burn the full write timeout, and since each beat holds the session lock, the
/// whole GUI hangs behind it. Observed 2026-07-31 — a stalled preset write left the heartbeat
/// failing every 2.25 s for the ~50 s until the user disconnected by hand. After
/// [`LOST_AFTER_BEATS`] consecutive failures the session is dropped and the frontend told via
/// `device-lost`.
pub fn spawn_heartbeat(app: tauri::AppHandle, session: Arc<Mutex<Option<Session>>>) {
    /// Consecutive failed beats before we declare the device gone. More than one so a single
    /// transient error can't disconnect a healthy session; small because each failure now costs a
    /// two-second timeout, and `Session::device_lost` short-circuits this on the first stall anyway.
    const LOST_AFTER_BEATS: u32 = 3;

    std::thread::spawn(move || {
        let mut failures = 0u32;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            // Poll under the lock, then release it before emitting.
            let mut lost = None;
            let pushes = {
                let Ok(mut guard) = session.lock() else {
                    continue;
                };
                match guard.as_mut() {
                    Some(s) => match s.poll_events() {
                        Ok(pushes) => {
                            failures = 0;
                            // A panel-side preset switch changes the editing context, same as
                            // goto_preset — the history belongs to the old preset. (The frontend's
                            // follow-up read reseeds.)
                            if pushes.iter().any(|p| {
                                matches!(
                                    p,
                                    fretwire_core::fretwire_data::stream::StatusPush::Preset(_)
                                )
                            }) {
                                s.clear_history();
                            }
                            pushes
                        }
                        Err(e) => {
                            failures += 1;
                            tracing::warn!(%e, failures, "keepalive failed");
                            // `device_lost` means the OUT endpoint has stalled, which no amount of
                            // waiting fixes — don't sit through the full failure count for it.
                            if s.device_lost() || failures >= LOST_AFTER_BEATS {
                                tracing::error!(
                                    "device stopped responding — dropping the session; the pedal \
                                     needs a power cycle"
                                );
                                failures = 0;
                                // Drops the `Session`, which closes and releases the interface.
                                *guard = None;
                                lost = Some(
                                    "The pedal stopped responding and the session was closed. \
                                     Power-cycle the HX device, then reconnect.",
                                );
                            }
                            Vec::new()
                        }
                    },
                    None => {
                        failures = 0;
                        continue;
                    }
                }
            };
            if let Some(msg) = lost
                && let Err(e) = app.emit("device-lost", msg)
            {
                tracing::warn!("failed to emit device-lost: {e}");
            }
            let dtos = crate::dto::push_dtos(&pushes);
            if !dtos.is_empty()
                && let Err(e) = app.emit("device-pushes", dtos)
            {
                tracing::warn!("failed to emit device-pushes: {e}");
            }
        }
    });
}

type R<T> = Result<T, String>;

/// Run `f` against the live session on a background thread, mapping errors to strings.
async fn run<T, F>(state: &AppState, f: F) -> R<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Session) -> fretwire_core::Result<T> + Send + 'static,
{
    let sess = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = sess
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?;
        let s = guard.as_mut().ok_or("not connected to the HX Stomp")?;
        f(s).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task error: {e}"))?
}

/// Build the wire DTO, stamping the session's edit-history state (which `From` can't see).
fn dto(s: &Session, p: &EditorPreset) -> PresetDto {
    let mut d = PresetDto::from(p);
    d.device_name = Some(s.device().name.to_string());
    d.device_matches = s.device_matches_preset(p);
    d.undo_depth = s.undo_depth() as i64;
    d.redo_depth = s.redo_depth() as i64;
    d.history = s.history_labels();
    d.history_cursor = s.history_cursor() as i64;
    d.dirty = s.dirty();
    d
}

/// Mutate via `f`, then re-read the preset so the frontend gets fresh, authoritative state.
async fn mutate<F>(state: &AppState, f: F) -> R<PresetDto>
where
    F: FnOnce(&mut Session) -> fretwire_core::Result<()> + Send + 'static,
{
    run(state, move |s| {
        f(s)?;
        let p = s.read_preset()?;
        Ok(dto(s, &p))
    })
    .await
}

/// [`mutate`] for an **undoable edit**: brackets `f` with `edit_begin(label)`/`edit_commit()` so
/// the post-edit state lands on the history timeline under that label. `label` is a builder run
/// against the *pre-edit* session, so it can name blocks/params via `slot_label`/`param_label`
/// (e.g. "Delete Simple Delay", not "Delete block (slot 4)").
async fn mutate_edit<L, F>(state: &AppState, label: L, f: F) -> R<PresetDto>
where
    L: FnOnce(&Session) -> String + Send + 'static,
    F: FnOnce(&mut Session) -> fretwire_core::Result<()> + Send + 'static,
{
    run(state, move |s| {
        let label = label(s);
        s.edit_begin(&label);
        // Close the history entry on the failure path too, or a refused edit leaves it dangling.
        let p = match f(s).and_then(|()| s.read_preset()) {
            Ok(p) => p,
            Err(e) => {
                s.edit_abort();
                return Err(e);
            }
        };
        s.edit_commit();
        Ok(dto(s, &p))
    })
    .await
}

/// Adapt a session method that already re-reads and returns an `EditorPreset`.
async fn returning<F>(state: &AppState, f: F) -> R<PresetDto>
where
    F: FnOnce(&mut Session) -> fretwire_core::Result<EditorPreset> + Send + 'static,
{
    run(state, move |s| {
        let p = f(s)?;
        Ok(dto(s, &p))
    })
    .await
}

/// [`returning`] for an **undoable edit**: brackets `f` (which re-reads internally) with
/// `edit_begin(label)`/`edit_commit()`. `label` builds against the pre-edit session (see
/// [`mutate_edit`]).
async fn returning_edit<L, F>(state: &AppState, label: L, f: F) -> R<PresetDto>
where
    L: FnOnce(&Session) -> String + Send + 'static,
    F: FnOnce(&mut Session) -> fretwire_core::Result<EditorPreset> + Send + 'static,
{
    run(state, move |s| {
        let label = label(s);
        s.edit_begin(&label);
        let p = match f(s) {
            Ok(p) => p,
            Err(e) => {
                s.edit_abort();
                return Err(e);
            }
        };
        s.edit_commit();
        Ok(dto(s, &p))
    })
    .await
}

// ---- reference data (first run) ----

/// Whether the Line 6 reference data has been imported. Cheap (one `stat` plus a dir listing), so
/// the frontend calls it on startup to decide between the editor and the first-run screen.
#[tauri::command]
pub fn data_status() -> DataStatusDto {
    fretwire_core::import::data_status().into()
}

/// Import the reference data from the user's own HX Edit installer or an extracted `res/` folder.
/// Unpacking an installer shells out to `7z` and walks the tree, so it runs off the UI thread.
#[tauri::command]
pub async fn import_data(source: String) -> R<ImportResultDto> {
    tauri::async_runtime::spawn_blocking(move || {
        fretwire_core::import::import_from(std::path::Path::new(&source))
            .map(ImportResultDto::from)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task error: {e}"))?
}

// ---- connection ----

/// USB enumeration only — does not claim the interface, safe to call anytime.
///
/// Returns what is actually plugged in rather than a bare yes/no, so the UI can name the pedal it
/// found. An empty list means nothing was seen.
#[tauri::command]
pub async fn detect() -> R<Vec<DetectedDeviceDto>> {
    tauri::async_runtime::spawn_blocking(|| {
        fretwire_core::fretwire_usb::present_devices()
            .map(|found| found.into_iter().map(DetectedDeviceDto::from).collect())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task error: {e}"))?
}

#[tauri::command]
pub fn is_connected(state: State<AppState>) -> bool {
    state.session.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// Open the session (idempotent) and return the current preset.
#[tauri::command]
pub async fn connect(state: State<'_, AppState>) -> R<PresetDto> {
    let sess = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || -> R<PresetDto> {
        let mut guard = sess
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?;
        if guard.is_none() {
            *guard = Some(Session::connect().map_err(|e| e.to_string())?);
        }
        let s = guard.as_mut().expect("just set");
        let preset = s.read_preset().map_err(|e| e.to_string())?;
        Ok(dto(s, &preset))
    })
    .await
    .map_err(|e| format!("task error: {e}"))?
}

/// Close the session (clean teardown) and drop it. No-op if not connected.
#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> R<()> {
    let sess = state.session.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let taken = sess
            .lock()
            .map_err(|_| "session lock poisoned".to_string())?
            .take();
        if let Some(mut s) = taken {
            s.close().map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("task error: {e}"))?
}

#[tauri::command]
pub async fn read_preset(state: State<'_, AppState>) -> R<PresetDto> {
    returning(&state, |s| s.read_preset()).await
}

// ---- undo / redo ----

/// Restore the pre-edit snapshot (op-21 write of the prior preset blob; edit buffer only).
#[tauri::command]
pub async fn undo(state: State<'_, AppState>) -> R<PresetDto> {
    returning(&state, |s| s.undo()).await
}

#[tauri::command]
pub async fn redo(state: State<'_, AppState>) -> R<PresetDto> {
    returning(&state, |s| s.redo()).await
}

/// Jump the edit buffer to history timeline entry `index` (history-pane click / A-B compare).
#[tauri::command]
pub async fn history_jump(state: State<'_, AppState>, index: usize) -> R<PresetDto> {
    returning(&state, move |s| s.history_jump(index)).await
}

// ---- block edits ----

/// `bypassed = true` engages bypass (block OFF); `false` activates the block.
#[tauri::command]
pub async fn set_bypass(state: State<'_, AppState>, slot: i64, bypassed: bool) -> R<PresetDto> {
    mutate_edit(
        &state,
        move |s| {
            format!(
                "{} {}",
                if bypassed { "Bypass" } else { "Enable" },
                s.slot_label(slot)
            )
        },
        move |s| s.set_enabled(slot, !bypassed),
    )
    .await
}

#[tauri::command]
pub async fn set_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    value: f32,
) -> R<PresetDto> {
    mutate_edit(
        &state,
        move |s| format!("Set {}", s.param_label(slot, false, param_index)),
        move |s| s.set_param(slot, param_index, value),
    )
    .await
}

/// Fire-and-forget param write for live audio feedback **while a slider drags** — no history
/// entry, no re-read (fast). The gesture ends with an ordinary `set_param`, which is the
/// authoritative commit. Mirrors HX Edit streaming values during a knob turn.
#[tauri::command]
pub async fn preview_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    value: f32,
) -> R<()> {
    run(&state, move |s| s.set_param(slot, param_index, value)).await
}

/// [`preview_param`] for the paired cab/IR params.
#[tauri::command]
pub async fn preview_paired_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    value: f32,
) -> R<()> {
    run(&state, move |s| {
        s.set_paired_param(slot, param_index, value)
    })
    .await
}

#[tauri::command]
pub async fn set_paired_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    value: f32,
) -> R<PresetDto> {
    mutate_edit(
        &state,
        move |s| format!("Set {}", s.param_label(slot, true, param_index)),
        move |s| s.set_paired_param(slot, param_index, value),
    )
    .await
}

/// Set an integer/enum/bool parameter by option index (sent on the wire as an int, not a float).
/// `paired` targets the block's cab/IR sub-model.
#[tauri::command]
pub async fn set_param_enum(
    state: State<'_, AppState>,
    slot: i64,
    paired: bool,
    param_index: i64,
    value: i64,
) -> R<PresetDto> {
    mutate_edit(
        &state,
        move |s| format!("Set {}", s.param_label(slot, paired, param_index)),
        move |s| s.set_param_enum(slot, paired, param_index, value),
    )
    .await
}

/// Swap the model at `slot` to `model_index` (a `Helix.sym` index). `paired_index` preserves an
/// amp's cab/IR pairing across the swap; pass `-1` for no pairing.
#[tauri::command]
pub async fn swap_model(
    state: State<'_, AppState>,
    slot: i64,
    model_index: i64,
    paired_index: i64,
) -> R<PresetDto> {
    // `returning_edit` + `read_preset_settled`, not `mutate_edit`: the device ACKs the swap before it
    // has rewritten the block's params, so a plain read-back can return the new model carrying the
    // old model's values (see `Session::read_preset_settled`).
    returning_edit(
        &state,
        move |s| {
            let mut label = format!(
                "{} \u{2192} {}",
                s.slot_label(slot),
                s.model_label(model_index)
            );
            // Name the cab too, so a cab-only change doesn't read as an amp swap.
            if paired_index >= 0 {
                label.push_str(&format!(" + {}", s.model_label(paired_index)));
            }
            label
        },
        move |s| {
            s.swap_model(slot, model_index, paired_index)?;
            s.read_preset_settled(slot)
        },
    )
    .await
}

#[tauri::command]
pub async fn add_block(
    state: State<'_, AppState>,
    model_index: i64,
    paired_index: i64,
) -> R<PresetDto> {
    returning_edit(
        &state,
        move |s| format!("Add {}", s.model_label(model_index)),
        move |s| s.add_block_append(model_index, paired_index),
    )
    .await
}

/// Add a block into a specific **empty grid slot** (the HX Edit "click an empty cell" flow),
/// rather than appending to the end of the chain.
#[tauri::command]
pub async fn add_block_at(
    state: State<'_, AppState>,
    slot: i64,
    model_index: i64,
    paired_index: i64,
) -> R<PresetDto> {
    // Settled read-back for the same reason as `swap_model` — op 39 fills the new block's params
    // after it ACKs.
    returning_edit(
        &state,
        move |s| format!("Add {}", s.model_label(model_index)),
        move |s| {
            s.add_block_at(slot, model_index, paired_index)?;
            s.read_preset_settled(slot)
        },
    )
    .await
}

#[tauri::command]
pub async fn delete_block(state: State<'_, AppState>, slot: i64) -> R<PresetDto> {
    returning_edit(
        &state,
        move |s| format!("Delete {}", s.slot_label(slot)),
        move |s| s.delete_block(slot),
    )
    .await
}

/// Reorder a block within the serial chain to order position `gap` (serial presets only).
#[tauri::command]
pub async fn reorder_block(state: State<'_, AppState>, src_slot: i64, gap: usize) -> R<PresetDto> {
    returning_edit(
        &state,
        move |s| format!("Move {}", s.slot_label(src_slot)),
        move |s| s.reorder_block(src_slot, gap),
    )
    .await
}

// ---- routing (split presets) ----

/// Move a block to the parallel (B) row or back to series (A), at insertion index `pos` in the row.
#[tauri::command]
pub async fn move_block_to_row(
    state: State<'_, AppState>,
    src_slot: i64,
    parallel: bool,
    pos: usize,
) -> R<PresetDto> {
    returning_edit(
        &state,
        move |s| format!("Move {}", s.slot_label(src_slot)),
        move |s| s.move_block_to_row(src_slot, parallel, pos),
    )
    .await
}

/// Move a block to the common (pre-split) region.
#[tauri::command]
pub async fn move_before_split(state: State<'_, AppState>, src_slot: i64) -> R<PresetDto> {
    returning_edit(
        &state,
        move |s| format!("Move {}", s.slot_label(src_slot)),
        move |s| s.move_before_split(src_slot),
    )
    .await
}

/// Place a block into an exact grid slot (the routing-grid primitive). `dst_slot` must be empty.
#[tauri::command]
pub async fn place_block(state: State<'_, AppState>, src_slot: i64, dst_slot: i64) -> R<PresetDto> {
    returning_edit(
        &state,
        move |s| format!("Move {}", s.slot_label(src_slot)),
        move |s| s.place_block(src_slot, dst_slot),
    )
    .await
}

/// Insert the dragged block before/after the occupied `dst_slot`, shifting neighbors to make room
/// (bubbled single op-43 moves; HX Edit's insert semantics for a drop onto a block).
#[tauri::command]
pub async fn insert_block(
    state: State<'_, AppState>,
    src_slot: i64,
    dst_slot: i64,
    before: bool,
) -> R<PresetDto> {
    returning_edit(
        &state,
        move |s| {
            format!(
                "Move {} {} {}",
                s.slot_label(src_slot),
                if before { "before" } else { "after" },
                s.slot_label(dst_slot)
            )
        },
        move |s| s.insert_block(src_slot, dst_slot, before),
    )
    .await
}

/// Move the split ("split") or join ("mixer") node to signal-flow column `pos` on the top row —
/// re-classifies blocks between common/path-A/common-after without moving any block. Goes through
/// the op-21 whole-preset write (edit buffer only).
#[tauri::command]
pub async fn set_node_pos(
    state: State<'_, AppState>,
    node: String,
    pos: i64,
    dsp: usize,
) -> R<PresetDto> {
    use fretwire_core::fretwire_data::stream::slot_kind;
    let kind = match node.as_str() {
        "split" => slot_kind::SPLIT,
        "mixer" => slot_kind::MIXER,
        other => {
            return Err(format!(
                "unknown node kind {other:?} (want \"split\" or \"mixer\")"
            ));
        }
    };
    returning_edit(
        &state,
        move |_| format!("Move {node} node \u{2192} col {pos}"),
        move |s| s.set_node_pos(dsp, kind, pos),
    )
    .await
}

/// Set the split type by swapping the split node's model (Y / A-B / Crossover / Dynamic).
#[tauri::command]
pub async fn set_split_type(
    state: State<'_, AppState>,
    split_slot: i64,
    model_index: i64,
) -> R<PresetDto> {
    returning_edit(
        &state,
        move |s| format!("Split type \u{2192} {}", s.model_label(model_index)),
        move |s| s.set_split_type(split_slot, model_index),
    )
    .await
}

// ---- controller assignments ----
//
// Two mechanisms, and the UI keeps them apart because the device does: a block's **bypass** on a
// footswitch is written with ops 56/57 and shows as `BlockDto::footswitch`, while a **parameter**
// under a controller is written with op 37 and shows in `PresetDto::assignments`. See
// `docs/protocol.md`, "Controller assignments — writing them".
//
// All four use `mutate_edit`, i.e. the ordinary immediate re-read, and **not**
// `read_preset_settled`. That was worth checking rather than assuming: a model swap ACKs before the
// device has rewritten the block's param area, which is why `swap_model` has to re-read until the
// decode stops changing. These do not — assigning, removing and unassigning all read back correctly
// on the very next read, three rounds in a row [solid — 2026-08-22].

/// Put the block in `slot`'s bypass on footswitch `switch` (**zero-based**, so 0 is FS1).
///
/// Re-sending it for a different switch **moves** the binding rather than adding a second one
/// [solid — verified live], so the UI's picker needs no unassign-then-assign dance.
#[tauri::command]
pub async fn assign_bypass(state: State<'_, AppState>, slot: i64, switch: i64) -> R<PresetDto> {
    mutate_edit(
        &state,
        move |s| format!("FS{} \u{2192} {}", switch + 1, s.slot_label(slot)),
        move |s| s.assign_bypass_to_switch(slot, switch),
    )
    .await
}

/// Take the block in `slot` off footswitch `switch` (zero-based). `switch` must be the one it is
/// actually on — the UI reads that from `BlockDto::footswitch`, which is one-based.
#[tauri::command]
pub async fn unassign_bypass(state: State<'_, AppState>, slot: i64, switch: i64) -> R<PresetDto> {
    mutate_edit(
        &state,
        move |s| format!("{} off FS{}", s.slot_label(slot), switch + 1),
        move |s| s.unassign_bypass_from_switch(slot, switch),
    )
    .await
}

/// Put a parameter under controller `source`, or remove it with source 0.
///
/// **The device does not range-check `source`** — ordinal 10 was accepted and silently did nothing,
/// because the controller table is ten long. Bounded here so a UI bug cannot make an assignment
/// that appears to work and isn't there.
#[tauri::command]
pub async fn assign_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    source: i64,
    paired: bool,
) -> R<PresetDto> {
    if !(0..=9).contains(&source) {
        return Err(format!(
            "controller {source} does not exist — sources run 0 (none) to 9"
        ));
    }
    mutate_edit(
        &state,
        move |s| {
            let p = s.param_label(slot, paired, param_index);
            if source == 0 {
                format!("Unassign {p}")
            } else {
                format!("{} \u{2192} {p}", crate::dto::source_name(source))
            }
        },
        move |s| s.assign_param(slot, paired, param_index, source),
    )
    .await
}

/// Move one end of an existing assignment's travel: `max = false` is Min, `true` is Max. The value
/// is in the parameter's own units, the same ones `set_param` takes.
#[tauri::command]
pub async fn set_assign_travel(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    max: bool,
    value: f32,
    paired: bool,
) -> R<PresetDto> {
    mutate_edit(
        &state,
        move |s| {
            format!(
                "{} {} of {}",
                if max { "Max" } else { "Min" },
                value,
                s.param_label(slot, paired, param_index)
            )
        },
        move |s| s.set_assign_travel(slot, paired, param_index, max, value),
    )
    .await
}

// ---- snapshots / preset navigation / persistence ----

#[tauri::command]
pub async fn set_snapshot(state: State<'_, AppState>, index: i64) -> R<PresetDto> {
    mutate(&state, move |s| s.set_snapshot(index)).await
}

#[tauri::command]
pub async fn goto_preset(state: State<'_, AppState>, bank: i64, preset: i64) -> R<PresetDto> {
    // Switching preset changes the editing context — the history belongs to the old one.
    mutate(&state, move |s| {
        s.clear_history();
        s.goto_preset(bank, preset)
    })
    .await
}

/// Save the edit buffer to flash at `bank`/`slot` under `name` (op 71).
#[tauri::command]
pub async fn save_preset(
    state: State<'_, AppState>,
    bank: i64,
    slot: i64,
    name: String,
) -> R<PresetDto> {
    mutate(&state, move |s| {
        check_cross_setlist_write(s, bank, "save_preset")?;
        s.save_preset(bank, slot, &name)
    })
    .await
}

/// Rename a preset in flash, name-only (op 6) — does not commit the edit buffer.
#[tauri::command]
pub async fn rename_preset(
    state: State<'_, AppState>,
    bank: i64,
    slot: i64,
    name: String,
) -> R<()> {
    run(&state, move |s| {
        check_cross_setlist_write(s, bank, "rename_preset")?;
        s.rename_preset(bank, slot, &name)
    })
    .await
}

/// Rename a snapshot of the current preset (op 89). An ordinary buffer edit — undoable, and
/// unsaved until the preset is saved.
#[tauri::command]
pub async fn rename_snapshot(state: State<'_, AppState>, index: i64, name: String) -> R<PresetDto> {
    mutate_edit(
        &state,
        move |_| format!("Rename snapshot {}", index + 1),
        move |s| s.rename_snapshot(index, &name),
    )
    .await
}

/// List the presets in one **setlist**. `bank` defaults to 0 — the HX Stomp's only list, and
/// Factory 1 on the Helix Floor.
#[tauri::command]
pub async fn list_presets(state: State<'_, AppState>, bank: Option<i64>) -> R<Vec<PresetListItem>> {
    let bank = bank.unwrap_or(0);
    run(&state, move |s| {
        let device = s.device();
        Ok(s.list_presets_in(bank)?
            .into_iter()
            .map(|(index, name)| PresetListItem {
                label: device.preset_label(index as i64),
                index: index as i64,
                name,
                // A live listing is already one setlist's worth; the caller passed the bank.
                bank,
                setlist: None,
            })
            .collect())
    })
    .await
}

/// Whether a **flash write** may target a setlist other than the one the device is in.
/// **Off unless `FRETWIRE_SETLISTS=1`.**
///
/// Browsing other setlists is no longer gated — the numbering that originally motivated the gate is
/// settled (a browse index is global, `bank * setlist_size + slot`; verified across all 1024 slots
/// of a Helix Floor against that unit's own `.hxb`, see `docs/helix-floor.md`), and browsing writes
/// nothing.
///
/// What stays gated is the one irreversible thing: **Save As into a setlist the device isn't in.**
/// Reading and navigating are recoverable; overwriting someone's preset is not, and the cross-setlist
/// write path has never run against a Helix Floor — the only device with setlists, and one that has
/// already been wedged once (`STATUS.md`, INCIDENT 2026-07-26). Lift this once a Floor gets through
/// a session cleanly.
fn cross_setlist_write_enabled() -> bool {
    matches!(
        std::env::var("FRETWIRE_SETLISTS").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// Reject a flash write aimed at a setlist the device isn't currently in, unless
/// [`cross_setlist_write_enabled`].
///
/// The device's own last-read identity is the reference, not anything the frontend tracks — the
/// point is to be a guard, and a guard that trusts its caller isn't one.
fn check_cross_setlist_write(s: &Session, bank: i64, what: &str) -> fretwire_core::Result<()> {
    if cross_setlist_write_enabled() {
        return Ok(());
    }
    // No read yet: nothing to compare against, and `check_preset_addr` still bounds the address.
    let Some(here) = s.last_identity().map(|i| i.bank) else {
        return Ok(());
    };
    if bank == here {
        return Ok(());
    }
    let name = |b: i64| {
        s.device()
            .setlist_names()
            .get(b as usize)
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("bank {b}"))
    };
    Err(fretwire_core::fretwire_data::Error::Stream(format!(
        "{what}: refusing to write into {} while the device is in {} — writing across setlists \
         is untested on this hardware. Set FRETWIRE_SETLISTS=1 to allow it.",
        name(bank),
        name(here),
    ))
    .into())
}

/// Whether the app may write into a setlist the device isn't in — so the UI can *show* the limit
/// (grey out Save As while browsing elsewhere) instead of letting the write fail at the wire.
#[tauri::command]
pub async fn cross_setlist_write_allowed() -> bool {
    cross_setlist_write_enabled()
}

/// The connected device's setlist names, in bank order — eight on the Helix Floor.
///
/// Browsing between them is **enabled**; only cross-setlist *writes* are gated, see
/// [`cross_setlist_write_enabled`]. The picker renders only for more than one setlist, so a Stomp
/// still shows none.
#[tauri::command]
pub async fn setlists(state: State<'_, AppState>) -> R<Vec<String>> {
    run(&state, |s| {
        Ok(s.device()
            .setlist_names()
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>())
    })
    .await
}

/// Resolve a user-typed backup path: `~/` and bare relative paths land in `$HOME`.
fn backup_path(p: &str) -> std::path::PathBuf {
    use std::path::PathBuf;
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let p = p.trim();
    if let Some(rest) = p.strip_prefix("~/") {
        return home().join(rest);
    }
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        home().join(pb)
    }
}

/// Export presets to a JSON file at `path` (relative/`~/` paths land in `$HOME`) — `banks` names
/// the setlists to walk. Reads only: flash is untouched, but the active-preset cursor sweeps every
/// listed setlist and any unsaved edit-buffer changes are reloaded from flash.
///
/// This is a setlist export, not a device backup — presets only, no globals and no IRs.
///
/// Progress streams to the frontend as `backup-progress` events; returns the count written. A
/// `cancel_export` in flight stops the sweep and writes what it has, which is why the count is
/// worth returning rather than assumed. The frontend re-reads the preset afterwards (the sweep
/// cleared the edit history).
#[tauri::command]
pub async fn export_setlists(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    banks: Vec<i64>,
) -> R<i64> {
    let target = backup_path(&path);
    let cancel = state.cancel_export.clone();
    cancel.store(false, Ordering::Relaxed);
    run(&state, move |s| {
        let backup = s.export_setlists(&banks, |p| {
            let _ = app.emit(
                "backup-progress",
                serde_json::json!({
                    "done": p.done, "total": p.total,
                    "bank": p.bank, "setlist": p.setlist, "name": p.name,
                }),
            );
            !cancel.load(Ordering::Relaxed)
        })?;
        std::fs::write(&target, backup.to_json()).map_err(|e| {
            fretwire_core::Error::Backup(format!("writing {}: {e}", target.display()))
        })?;
        Ok(backup.presets.len() as i64)
    })
    .await
}

/// Call off an export sweep in flight. Takes effect at the next preset boundary; the file is still
/// written, holding everything read up to that point.
#[tauri::command]
pub async fn cancel_export(state: State<'_, AppState>) -> R<()> {
    state.cancel_export.store(true, Ordering::Relaxed);
    Ok(())
}

/// List a backup file's contents (indices + names) so the restore dialog can offer a picker.
/// Pure file I/O — works without a device.
#[tauri::command]
pub async fn backup_show(path: String) -> R<Vec<PresetListItem>> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = backup_path(&path);
        let text = std::fs::read_to_string(&target)
            .map_err(|e| format!("reading {}: {e}", target.display()))?;
        let backup = fretwire_core::backup::Backup::from_json(&text).map_err(|e| e.to_string())?;
        Ok(backup
            .presets
            .into_iter()
            .map(|p| PresetListItem {
                // An export file records no device banking, and the file may be from another unit,
                // so its entries stay on slot numbers.
                label: None,
                index: p.index,
                bank: p.bank,
                setlist: backup
                    .setlists
                    .iter()
                    .find(|(b, _)| *b == p.bank)
                    .map(|(_, n)| n.clone()),
                name: p.name,
            })
            .collect())
    })
    .await
    .map_err(|e| format!("task error: {e}"))?
}

/// Restore one preset from a backup file into setlist `slot` — **overwrites the slot in flash**
/// (op-21 edit-buffer write + op-71 save). Clears the edit history (new editing context) and
/// returns the re-read preset.
#[tauri::command]
pub async fn restore_preset(
    state: State<'_, AppState>,
    path: String,
    index: i64,
    slot: i64,
    bank: i64,
) -> R<PresetDto> {
    let target = backup_path(&path);
    run(&state, move |s| {
        let text = std::fs::read_to_string(&target).map_err(|e| {
            fretwire_core::Error::Backup(format!("reading {}: {e}", target.display()))
        })?;
        let backup = fretwire_core::backup::Backup::from_json(&text)?;
        let entry = backup.preset(bank, index).ok_or_else(|| {
            fretwire_core::Error::Backup(format!(
                "export file has no preset at bank {bank} slot {index}"
            ))
        })?;
        let p = s.restore_preset(&entry.raw, bank, slot, &entry.name)?;
        Ok(dto(s, &p))
    })
    .await
}

/// Copy the currently-loaded preset into the app's paste buffer. Reads only. Returns the name, so
/// the UI can say what is on the clipboard.
#[tauri::command]
pub async fn copy_preset(state: State<'_, AppState>) -> R<String> {
    let clipboard = state.clipboard.clone();
    run(&state, move |s| {
        let raw = s.read_preset_raw()?;
        let name = s
            .last_identity()
            .map(|i| i.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "preset".to_string());
        if let Ok(mut c) = clipboard.lock() {
            *c = Some(PresetClip {
                raw,
                name: name.clone(),
            });
        }
        Ok(name)
    })
    .await
}

/// Paste the copied preset over the currently-loaded one — **into the edit buffer, not flash**, the
/// same as every other structural edit. The user saves when they mean to, and undo steps back out
/// of it. Mirrors HX Edit's copy-preset / paste-onto-a-slot, which is otherwise a manual rebuild.
#[tauri::command]
pub async fn paste_preset(state: State<'_, AppState>) -> R<PresetDto> {
    let Some(PresetClip { raw, name }) = state.clipboard.lock().ok().and_then(|c| c.clone()) else {
        return Err("nothing copied yet — use Copy on a preset first".into());
    };
    mutate_edit(
        &state,
        move |_| format!("Paste {name}"),
        move |s| {
            let ps = fretwire_core::fretwire_data::stream::PresetStream::parse(&raw)?;
            // A Floor preset carries two DSPs' worth of slots and a Stomp preset one, so pasting
            // across device models would write a shape the pedal cannot hold. Catch it here rather
            // than let op 21 find out.
            if let Ok(p) = s.catalog().load_preset(&raw)
                && s.device_matches_preset(&p) == Some(false)
            {
                return Err(fretwire_core::Error::Rejected(format!(
                    "that preset came from a {} — this is a {}",
                    p.device_model.as_deref().unwrap_or("different device"),
                    s.device().name,
                )));
            }
            s.write_preset(ps.to_blob())
        },
    )
    .await
}

/// Whether anything is on the paste buffer, and what it is called — so the UI can label/enable the
/// Paste button. No device needed.
#[tauri::command]
pub fn clipboard_preset(state: State<'_, AppState>) -> Option<String> {
    state
        .clipboard
        .lock()
        .ok()
        .and_then(|c| c.as_ref().map(|p| p.name.clone()))
}

/// Copy one block — its model, paired cab, bypass state and every parameter value. Reads only.
#[tauri::command]
pub async fn copy_block(state: State<'_, AppState>, slot: i64) -> R<String> {
    let clip = state.block_clipboard.clone();
    run(&state, move |s| {
        let preset = s.read_preset()?;
        let b = preset.block(slot).ok_or_else(|| {
            fretwire_core::Error::Rejected(format!("slot {slot} has no block to copy"))
        })?;
        let model_index = b.model_index.ok_or_else(|| {
            fretwire_core::Error::Rejected(format!(
                "{} has no model reference to copy (it is a routing node, not a block)",
                b.model_name
            ))
        })?;
        let clipped = BlockClip {
            name: b.user_label.clone().unwrap_or_else(|| b.model_name.clone()),
            model_index,
            paired_index: b.paired_index.unwrap_or(-1),
            bypassed: b.bypassed.unwrap_or(false),
            params: b.params.iter().map(|p| (p.index as i64, p.value)).collect(),
            paired_params: b
                .paired_params
                .iter()
                .map(|p| (p.index as i64, p.value))
                .collect(),
        };
        let name = clipped.name.clone();
        if let Ok(mut c) = clip.lock() {
            *c = Some(clipped);
        }
        Ok(name)
    })
    .await
}

/// Paste the copied block into `slot`, replacing whatever is there.
///
/// Built from the **surgical** ops — one `swap_model` then one `set_value` per parameter — rather
/// than splicing the blob and doing an op-21 whole-preset write. Slower on the wire (a couple of
/// hundred ms) and worth it: op 21 is the operation every device lockup on record has come from,
/// and a convenience feature has no business going near it while that is unexplained.
#[tauri::command]
pub async fn paste_block(state: State<'_, AppState>, slot: i64) -> R<PresetDto> {
    let Some(clip) = state.block_clipboard.lock().ok().and_then(|c| c.clone()) else {
        return Err("no block copied yet — use Copy on a block first".into());
    };
    let label = clip.name.clone();
    mutate_edit(
        &state,
        move |_| format!("Paste {label}"),
        move |s| {
            // The swap resets the block to the new model's defaults, so every value has to be
            // replayed afterwards — that ordering is not optional.
            s.swap_model(slot, clip.model_index, clip.paired_index)?;
            // …and the re-read is not optional either. `set_param` clamps to the param's declared
            // range, which it looks up in the *cached* preset, and the swap leaves that cache
            // describing the model that was there before. Without this, pasting a cab clamped its
            // Mic index 11 to 1 and its High Cut 20100 Hz to 1.0 — against the old block's ranges.
            s.read_preset()?;
            for (index, value) in clip.params {
                s.set_param_value(slot, false, index, value)?;
            }
            for (index, value) in clip.paired_params {
                s.set_param_value(slot, true, index, value)?;
            }
            s.set_enabled(slot, !clip.bypassed)
        },
    )
    .await
}

/// What is on the block paste buffer, if anything. No device needed.
#[tauri::command]
pub fn clipboard_block(state: State<'_, AppState>) -> Option<String> {
    state
        .block_clipboard
        .lock()
        .ok()
        .and_then(|c| c.as_ref().map(|b| b.name.clone()))
}

/// The split types available for the split node (Y / A-B / Crossover / Dynamic). Static catalog
/// data — no device needed.
#[tauri::command]
pub fn split_types() -> Vec<SplitTypeDto> {
    fretwire_core::editor::SPLIT_TYPES
        .iter()
        .map(|(index, sym, label)| SplitTypeDto {
            index: *index,
            symbolic_id: sym.to_string(),
            label: label.to_string(),
        })
        .collect()
}

// ---- model picker (catalog, no device mutation) ----

#[tauri::command]
pub async fn categories(state: State<'_, AppState>) -> R<Vec<CategoryDto>> {
    // The colour is resolved inside the session closure because it needs the catalog; formatting it
    // as CSS hex is the UI's dialect, so that happens here rather than in `fretwire-core`.
    run(&state, |s| {
        Ok(s.catalog()
            .categories()
            .into_iter()
            .map(|(id, name)| {
                (
                    id,
                    name.to_string(),
                    s.catalog().category_color(id).map(|c| format!("#{c:06x}")),
                )
            })
            .collect::<Vec<_>>())
    })
    .await
    .map(|v| {
        v.into_iter()
            .map(|(id, name, color)| CategoryDto { id, name, color })
            .collect()
    })
}

#[tauri::command]
pub async fn models_in_category(
    state: State<'_, AppState>,
    category: i64,
    variant: Option<String>,
) -> R<Vec<ModelChoiceDto>> {
    run(&state, move |s| {
        Ok(s.catalog().models_in_category(category, variant.as_deref()))
    })
    .await
    .map(|v| v.iter().map(ModelChoiceDto::from).collect())
}

/// Which preset-numbering form the pedal's own screen is set to — `"flat"` (`000`-`127`) or
/// `"banked"` (`01A`-`32D`). **Reads only.**
///
/// Setting id 27, measured on a physical HX Stomp (see `docs/protocol.md`). `None` when the device
/// doesn't answer that id, which is the honest answer for any device we haven't checked: the UI
/// keeps its own preference rather than adopting a guess.
#[tauri::command]
pub async fn device_numbering(state: State<'_, AppState>) -> R<Option<String>> {
    run(&state, |s| {
        Ok(s.read_setting(SETTING_PRESET_NUMBERING)?
            .and_then(|v| v.as_bool())
            .map(|flat| numbering_word(flat).to_string()))
    })
    .await
}

/// Setting id for the preset-numbering form. See `docs/protocol.md`.
const SETTING_PRESET_NUMBERING: i64 = 27;

/// The two words [`device_numbering`] can return. The UI matches on these exactly and ignores
/// anything else, so a typo here would silently do nothing rather than fail — hence the test.
fn numbering_word(flat: bool) -> &'static str {
    if flat { "flat" } else { "banked" }
}

#[cfg(test)]
mod numbering_tests {
    use super::numbering_word;

    /// Pinned against `ui/src/lib/numbering.svelte.js`, which compares against these two literals,
    /// and against the mock backend's `device_numbering` (checked in `ui/tests/ir-mock.mjs`).
    #[test]
    fn the_words_the_ui_matches_on() {
        assert_eq!(numbering_word(true), "flat");
        assert_eq!(numbering_word(false), "banked");
    }
}

// ---- device settings (globals) ----
//
// These are **not** preset edits: they change the pedal itself, take effect immediately and have no
// edit-buffer stage, so none of them go on the undo timeline and none return a `PresetDto`. The
// same shape as the IR commands, for the same reason.

/// The device's global settings. **Reads only.**
///
/// `all = false` reads just the identified ids — 27 reads, instant. `all = true` sweeps the whole
/// answering space (nothing above 226 responds on an HX Stomp) and includes the unidentified ids as
/// `kind: "raw"`, which is what makes the panel usable for mapping the rest.
#[tauri::command]
pub async fn settings_read(state: State<'_, AppState>, all: bool) -> R<Vec<SettingDto>> {
    use fretwire_core::fretwire_protocol::settings;
    run(&state, move |s| {
        let found = if all {
            s.scan_settings(0..=SETTINGS_MAX_ID)
        } else {
            // Ask for exactly the ids we can name, rather than sweeping and filtering: an
            // unimplemented id on some other device is then simply absent, not an error.
            s.scan_settings(settings::SETTINGS.iter().map(|d| d.id))
        };
        let mut rows: Vec<SettingDto> = found
            .iter()
            .map(|(id, v)| SettingDto::new(*id, v))
            .collect();
        // The panel renders each group in the order the rows arrive, so this is where the pedal's
        // own menu order gets applied. `settings::SETTINGS` stays in id order for the sake of
        // reading it; an id nobody has placed in a menu keeps its numeric position, after the
        // placed ones.
        rows.sort_by_key(|r| (settings::menu_rank(r.id), r.id));
        Ok(rows)
    })
    .await
}

/// Write one global setting and return it as the device reports it back.
///
/// Refuses any id we have not identified — see `settings::is_writable`. The value is sent in
/// whatever type the device already holds (`Session::set_setting_num` reads it first), because a
/// type mismatch is refused with `-3`.
#[tauri::command]
pub async fn settings_write(state: State<'_, AppState>, id: i64, value: f64) -> R<SettingDto> {
    use fretwire_core::fretwire_protocol::settings;
    if !settings::is_writable(id) {
        // Not a device error — a deliberate refusal, so it reads as one.
        return Err(format!(
            "setting {id} is not one fretwire has identified, so it will not be written. \
             Change it on the pedal and use `fretwire settings-diff` to name it first."
        ));
    }
    run(&state, move |s| {
        let after = s.set_setting_num(id, value)?.ok_or_else(|| {
            fretwire_core::Error::from(fretwire_core::fretwire_data::Error::Stream(format!(
                "setting {id} accepted the write but reports no value back"
            )))
        })?;
        Ok(SettingDto::new(id, &after))
    })
    .await
}

/// Top of the answering id space on an HX Stomp — 226 is the highest that responds, and the sweep
/// is cheap enough (~0.8 ms a read) that rounding up costs nothing.
const SETTINGS_MAX_ID: i64 = 260;

// ---- user IR slots ----
//
// These do not touch the preset, so unlike every other mutating command here they return an IR
// listing rather than a `PresetDto`, and none of them go on the undo timeline: an IR write is a
// flash write with no edit-buffer stage to roll back.

/// The device's IR directory — the populated slots, in one request. **Reads only.**
#[tauri::command]
pub async fn ir_list(state: State<'_, AppState>) -> R<Vec<IrSlotDto>> {
    run(&state, |s| {
        Ok(s.ir_directory()?.iter().map(IrSlotDto::from).collect())
    })
    .await
}

/// Every slot including the empty ones, one select each. Slower; for the "show empty slots" view.
#[tauri::command]
pub async fn ir_scan(state: State<'_, AppState>) -> R<Vec<IrSlotDto>> {
    run(&state, |s| {
        Ok(s.ir_scan()?.iter().map(IrSlotDto::from).collect())
    })
    .await
}

/// Write slot `slot` out to `path` as a 32-bit float, 48 kHz mono WAV. **Reads the device only.**
#[tauri::command]
pub async fn ir_export(state: State<'_, AppState>, slot: i64, path: String) -> R<String> {
    run(&state, move |s| {
        let Some((info, blob)) = s.ir_export(slot)? else {
            return Err(fretwire_core::fretwire_data::Error::Stream(format!(
                "IR slot {slot} is empty"
            ))
            .into());
        };
        std::fs::write(&path, fretwire_core::fretwire_data::ir::to_wav(&blob))
            .map_err(fretwire_core::fretwire_data::Error::Io)?;
        Ok(info.name)
    })
    .await
}

/// Upload the WAV at `path` into slot `slot`. **Writes device flash.**
///
/// The sample rate is not converted, so a file that is not 48 kHz is refused unless `force` — it
/// would play short and bright. The frontend confirms an occupied slot before setting `overwrite`.
#[tauri::command]
pub async fn ir_upload(
    state: State<'_, AppState>,
    slot: i64,
    path: String,
    name: Option<String>,
    overwrite: bool,
    force: bool,
) -> R<Vec<IrSlotDto>> {
    run(&state, move |s| {
        let bytes = std::fs::read(&path).map_err(fretwire_core::fretwire_data::Error::Io)?;
        let (blob, rate) = fretwire_core::fretwire_data::ir::from_wav(&bytes)
            .map_err(|e| fretwire_core::fretwire_data::Error::Stream(format!("{path}: {e}")))?;
        if rate != fretwire_core::fretwire_data::ir::IR_SAMPLE_RATE && !force {
            return Err(fretwire_core::fretwire_data::Error::Stream(format!(
                "that file is {rate} Hz and the device runs at {} Hz. Nothing here resamples, so \
                 it would play short and bright — convert it first",
                fretwire_core::fretwire_data::ir::IR_SAMPLE_RATE
            ))
            .into());
        }
        let name = name.unwrap_or_else(|| {
            std::path::Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        s.ir_upload(slot, &name, &blob, overwrite)?;
        Ok(s.ir_directory()?.iter().map(IrSlotDto::from).collect())
    })
    .await
}

/// Empty a slot. **Writes device flash**, and there is no undo.
#[tauri::command]
pub async fn ir_delete(state: State<'_, AppState>, slot: i64) -> R<Vec<IrSlotDto>> {
    run(&state, move |s| {
        s.ir_delete(slot)?;
        Ok(s.ir_directory()?.iter().map(IrSlotDto::from).collect())
    })
    .await
}

/// Rename the IR in a slot. **Writes device flash** — the name only.
#[tauri::command]
pub async fn ir_rename(state: State<'_, AppState>, slot: i64, name: String) -> R<Vec<IrSlotDto>> {
    run(&state, move |s| {
        s.ir_rename(slot, &name)?;
        Ok(s.ir_directory()?.iter().map(IrSlotDto::from).collect())
    })
    .await
}
