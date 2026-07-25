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
    CategoryDto, DataStatusDto, ImportResultDto, ModelChoiceDto, PresetDto, PresetListItem,
    SplitTypeDto,
};
use fretwire_core::{EditorPreset, Session};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

#[derive(Default)]
pub struct AppState {
    pub session: Arc<Mutex<Option<Session>>>,
}

/// Spawn the keepalive heartbeat. While a session is open, this beats every 250 ms — same cadence as
/// the iced GUI — sending an idle frame on each channel and draining the device's queued
/// status-pushes/meters. Without it, those pile up between commands and desync the next read (a model
/// swap then times out and every later command hangs on the lock). Runs for the app's lifetime;
/// no-ops while disconnected.
///
/// Device-originated changes (footswitch bypass, panel snapshot/preset switch) are forwarded to the
/// frontend as a `device-pushes` event so the GUI follows the hardware live.
pub fn spawn_heartbeat(app: tauri::AppHandle, session: Arc<Mutex<Option<Session>>>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            // Poll under the lock, then release it before emitting.
            let pushes = {
                let Ok(mut guard) = session.lock() else {
                    continue;
                };
                match guard.as_mut() {
                    Some(s) => {
                        let pushes = s.poll_events().unwrap_or_default();
                        // A panel-side preset switch changes the editing context, same as goto_preset —
                        // the history belongs to the old preset. (The frontend's follow-up read reseeds.)
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
                    None => continue,
                }
            };
            let dtos = crate::dto::push_dtos(&pushes);
            if !dtos.is_empty() {
                if let Err(e) = app.emit("device-pushes", dtos) {
                    tracing::warn!("failed to emit device-pushes: {e}");
                }
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
        f(s)?;
        let p = s.read_preset()?;
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
        let p = f(s)?;
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
#[tauri::command]
pub async fn detect() -> R<bool> {
    tauri::async_runtime::spawn_blocking(|| {
        fretwire_core::fretwire_usb::hx_device_present().map_err(|e| e.to_string())
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
    mutate(&state, move |s| s.save_preset(bank, slot, &name)).await
}

/// Rename a preset in flash, name-only (op 6) — does not commit the edit buffer.
#[tauri::command]
pub async fn rename_preset(
    state: State<'_, AppState>,
    bank: i64,
    slot: i64,
    name: String,
) -> R<()> {
    run(&state, move |s| s.rename_preset(bank, slot, &name)).await
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

#[tauri::command]
pub async fn list_presets(state: State<'_, AppState>) -> R<Vec<PresetListItem>> {
    run(&state, |s| s.list_presets()).await.map(|v| {
        v.into_iter()
            .map(|(index, name)| PresetListItem {
                index: index as i64,
                name,
            })
            .collect()
    })
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

/// Back up the whole setlist to a JSON file at `path` (relative/`~/` paths land in `$HOME`).
/// Reads only — flash is untouched — but the active-preset cursor sweeps the setlist and any
/// unsaved edit-buffer changes are reloaded from flash. Progress streams to the frontend as
/// `backup-progress` events `{done, total, name}`; returns the count written. The frontend
/// re-reads the preset afterwards (the sweep cleared the edit history).
#[tauri::command]
pub async fn backup_setlist(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> R<i64> {
    let target = backup_path(&path);
    run(&state, move |s| {
        let backup = s.backup_setlist(|done, total, name| {
            let _ = app.emit(
                "backup-progress",
                serde_json::json!({ "done": done, "total": total, "name": name }),
            );
        })?;
        std::fs::write(&target, backup.to_json()).map_err(|e| {
            fretwire_core::Error::Backup(format!("writing {}: {e}", target.display()))
        })?;
        Ok(backup.presets.len() as i64)
    })
    .await
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
                index: p.index,
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
) -> R<PresetDto> {
    let target = backup_path(&path);
    run(&state, move |s| {
        let text = std::fs::read_to_string(&target).map_err(|e| {
            fretwire_core::Error::Backup(format!("reading {}: {e}", target.display()))
        })?;
        let backup = fretwire_core::backup::Backup::from_json(&text)?;
        let entry = backup.preset(index).ok_or_else(|| {
            fretwire_core::Error::Backup(format!("backup has no preset at index {index}"))
        })?;
        let p = s.restore_preset(&entry.raw, slot, &entry.name)?;
        Ok(dto(s, &p))
    })
    .await
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
    run(&state, |s| Ok(s.catalog().categories()))
        .await
        .map(|v| {
            v.into_iter()
                .map(|(id, name)| CategoryDto {
                    id,
                    name: name.to_string(),
                })
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
