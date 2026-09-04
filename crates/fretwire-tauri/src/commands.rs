//! The Tauri binding of the command surface: one-line `#[tauri::command]` wrappers over every
//! `fretwire_commands` command, plus [`TauriSink`] carrying its events onto the webview and the
//! one desktop-only command (`open_url`) that has no place in a daemon. The command
//! bodies, `AppState`, the DTOs and the heartbeat all live in `fretwire-commands` so a second
//! transport (`fretwire-serve`, see `docs/serve-mode.md`) can consume them unchanged.
//!
//! Keep the wrappers signature-identical to the lifted functions: the `#[tauri::command]` macro
//! derives the wire argument names (camelCased) from these parameter names, and the frontend's
//! `invoke()` calls depend on them.

use fretwire_commands::R;
use fretwire_commands::dto::{
    BackupFileDto, BackupInfoDto, BackupSummaryDto, CategoryDto, DataStatusDto, DetectedDeviceDto,
    ImportResultDto, IrFileDto, IrSlotDto, ModelChoiceDto, PresetDto, PresetListItem,
    RestoreReportDto, SettingDto, SplitTypeDto, UpdateStatusDto,
};
use fretwire_commands::events::{Event, EventSink};
use tauri::{Emitter, State};

pub use fretwire_commands::AppState;

/// [`EventSink`] over the Tauri event system (a newtype — the orphan rule keeps the impl off
/// `AppHandle` itself). Emit failures are logged, not surfaced: the producers have nowhere to
/// report them, exactly as before the lift.
pub struct TauriSink(pub tauri::AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: Event) {
        if let Err(e) = self.0.emit(event.name(), event.payload()) {
            tracing::warn!("failed to emit {}: {e}", event.name());
        }
    }
}

/// Spawn the keepalive heartbeat, emitting to the webview. See `fretwire_commands::spawn_heartbeat`.
pub fn spawn_heartbeat(
    app: tauri::AppHandle,
    session: std::sync::Arc<std::sync::Mutex<Option<fretwire_core::Session>>>,
) {
    fretwire_commands::spawn_heartbeat(TauriSink(app), session);
}

// ---- reference data (first run) ----

#[tauri::command]
pub fn data_status() -> DataStatusDto {
    fretwire_commands::data_status()
}

#[tauri::command]
pub async fn import_data(source: String) -> R<ImportResultDto> {
    fretwire_commands::import_data(source).await
}

// ---- update check ----

#[tauri::command]
pub fn update_status() -> UpdateStatusDto {
    fretwire_commands::update_status()
}

#[tauri::command]
pub async fn update_check(force: bool) -> R<UpdateStatusDto> {
    fretwire_commands::update_check(force).await
}

#[tauri::command]
pub async fn update_pref(enabled: bool) -> R<UpdateStatusDto> {
    fretwire_commands::update_pref(enabled).await
}

/// Open the release page in the user's browser. Tauri-only on purpose — it is not in
/// `fretwire-commands`, because under serve mode the "backend" is a daemon on another machine
/// and a browser must not open over there; the frontend uses a plain link in that case.
///
/// Restricted to this project's release pages: the webview is the only caller, but a command
/// that hands arbitrary strings to `xdg-open` is a wider door than the feature needs.
#[tauri::command]
pub fn open_url(url: String) -> R<()> {
    const ALLOWED: &str = "https://github.com/john-baxter-dev/fretwire/releases";
    if !url.starts_with(ALLOWED) {
        return Err(format!(
            "refusing to open {url}: not a fretwire release page"
        ));
    }
    std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("xdg-open: {e}"))
}

// ---- connection ----

#[tauri::command]
pub async fn detect() -> R<Vec<DetectedDeviceDto>> {
    fretwire_commands::detect().await
}

#[tauri::command]
pub fn is_connected(state: State<AppState>) -> bool {
    fretwire_commands::is_connected(&state)
}

#[tauri::command]
pub async fn connect(state: State<'_, AppState>) -> R<PresetDto> {
    fretwire_commands::connect(&state).await
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> R<()> {
    fretwire_commands::disconnect(&state).await
}

#[tauri::command]
pub async fn read_preset(state: State<'_, AppState>) -> R<PresetDto> {
    fretwire_commands::read_preset(&state).await
}

// ---- undo / redo ----

#[tauri::command]
pub async fn undo(state: State<'_, AppState>) -> R<PresetDto> {
    fretwire_commands::undo(&state).await
}

#[tauri::command]
pub async fn redo(state: State<'_, AppState>) -> R<PresetDto> {
    fretwire_commands::redo(&state).await
}

#[tauri::command]
pub async fn history_jump(state: State<'_, AppState>, index: usize) -> R<PresetDto> {
    fretwire_commands::history_jump(&state, index).await
}

// ---- block edits ----

#[tauri::command]
pub async fn set_bypass(state: State<'_, AppState>, slot: i64, bypassed: bool) -> R<PresetDto> {
    fretwire_commands::set_bypass(&state, slot, bypassed).await
}

#[tauri::command]
pub async fn set_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    value: f32,
) -> R<PresetDto> {
    fretwire_commands::set_param(&state, slot, param_index, value).await
}

#[tauri::command]
pub async fn preview_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    value: f32,
) -> R<()> {
    fretwire_commands::preview_param(&state, slot, param_index, value).await
}

#[tauri::command]
pub async fn preview_paired_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    value: f32,
) -> R<()> {
    fretwire_commands::preview_paired_param(&state, slot, param_index, value).await
}

#[tauri::command]
pub async fn set_paired_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    value: f32,
) -> R<PresetDto> {
    fretwire_commands::set_paired_param(&state, slot, param_index, value).await
}

#[tauri::command]
pub async fn set_param_enum(
    state: State<'_, AppState>,
    slot: i64,
    paired: bool,
    param_index: i64,
    value: i64,
) -> R<PresetDto> {
    fretwire_commands::set_param_enum(&state, slot, paired, param_index, value).await
}

#[tauri::command]
pub async fn swap_model(
    state: State<'_, AppState>,
    slot: i64,
    model_index: i64,
    paired_index: i64,
) -> R<PresetDto> {
    fretwire_commands::swap_model(&state, slot, model_index, paired_index).await
}

#[tauri::command]
pub async fn add_block(
    state: State<'_, AppState>,
    model_index: i64,
    paired_index: i64,
) -> R<PresetDto> {
    fretwire_commands::add_block(&state, model_index, paired_index).await
}

#[tauri::command]
pub async fn add_block_at(
    state: State<'_, AppState>,
    slot: i64,
    model_index: i64,
    paired_index: i64,
) -> R<PresetDto> {
    fretwire_commands::add_block_at(&state, slot, model_index, paired_index).await
}

#[tauri::command]
pub async fn delete_block(state: State<'_, AppState>, slot: i64) -> R<PresetDto> {
    fretwire_commands::delete_block(&state, slot).await
}

#[tauri::command]
pub async fn clear_preset(state: State<'_, AppState>) -> R<PresetDto> {
    fretwire_commands::clear_preset(&state).await
}

#[tauri::command]
pub async fn revert_preset(state: State<'_, AppState>) -> R<PresetDto> {
    fretwire_commands::revert_preset(&state).await
}

#[tauri::command]
pub async fn reorder_block(state: State<'_, AppState>, src_slot: i64, gap: usize) -> R<PresetDto> {
    fretwire_commands::reorder_block(&state, src_slot, gap).await
}

// ---- routing (split presets) ----

#[tauri::command]
pub async fn move_block_to_row(
    state: State<'_, AppState>,
    src_slot: i64,
    parallel: bool,
    pos: usize,
) -> R<PresetDto> {
    fretwire_commands::move_block_to_row(&state, src_slot, parallel, pos).await
}

#[tauri::command]
pub async fn move_before_split(state: State<'_, AppState>, src_slot: i64) -> R<PresetDto> {
    fretwire_commands::move_before_split(&state, src_slot).await
}

#[tauri::command]
pub async fn place_block(state: State<'_, AppState>, src_slot: i64, dst_slot: i64) -> R<PresetDto> {
    fretwire_commands::place_block(&state, src_slot, dst_slot).await
}

#[tauri::command]
pub async fn insert_block(
    state: State<'_, AppState>,
    src_slot: i64,
    dst_slot: i64,
    before: bool,
) -> R<PresetDto> {
    fretwire_commands::insert_block(&state, src_slot, dst_slot, before).await
}

#[tauri::command]
pub async fn set_node_pos(
    state: State<'_, AppState>,
    node: String,
    pos: i64,
    dsp: usize,
) -> R<PresetDto> {
    fretwire_commands::set_node_pos(&state, node, pos, dsp).await
}

#[tauri::command]
pub async fn set_split_type(
    state: State<'_, AppState>,
    split_slot: i64,
    model_index: i64,
) -> R<PresetDto> {
    fretwire_commands::set_split_type(&state, split_slot, model_index).await
}

// ---- controller assignments ----

#[tauri::command]
pub async fn assign_bypass(state: State<'_, AppState>, slot: i64, switch: i64) -> R<PresetDto> {
    fretwire_commands::assign_bypass(&state, slot, switch).await
}

#[tauri::command]
pub async fn unassign_bypass(state: State<'_, AppState>, slot: i64, switch: i64) -> R<PresetDto> {
    fretwire_commands::unassign_bypass(&state, slot, switch).await
}

#[tauri::command]
pub async fn set_switch_label(
    state: State<'_, AppState>,
    switch: i64,
    label: Option<String>,
) -> R<PresetDto> {
    fretwire_commands::set_switch_label(&state, switch, label).await
}

#[tauri::command]
pub async fn set_switch_momentary(
    state: State<'_, AppState>,
    switch: i64,
    momentary: bool,
) -> R<PresetDto> {
    fretwire_commands::set_switch_momentary(&state, switch, momentary).await
}

#[tauri::command]
pub async fn set_switch_color(
    state: State<'_, AppState>,
    switch: i64,
    color: Option<i64>,
) -> R<PresetDto> {
    fretwire_commands::set_switch_color(&state, switch, color).await
}

#[tauri::command]
pub async fn assign_param(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    source: i64,
    paired: bool,
) -> R<PresetDto> {
    fretwire_commands::assign_param(&state, slot, param_index, source, paired).await
}

#[tauri::command]
pub async fn set_assign_travel(
    state: State<'_, AppState>,
    slot: i64,
    param_index: i64,
    max: bool,
    value: f32,
    paired: bool,
) -> R<PresetDto> {
    fretwire_commands::set_assign_travel(&state, slot, param_index, max, value, paired).await
}

// ---- snapshots / preset navigation / persistence ----

#[tauri::command]
pub async fn set_snapshot(state: State<'_, AppState>, index: i64) -> R<PresetDto> {
    fretwire_commands::set_snapshot(&state, index).await
}

#[tauri::command]
pub async fn goto_preset(state: State<'_, AppState>, bank: i64, preset: i64) -> R<PresetDto> {
    fretwire_commands::goto_preset(&state, bank, preset).await
}

#[tauri::command]
pub async fn save_preset(
    state: State<'_, AppState>,
    bank: i64,
    slot: i64,
    name: String,
) -> R<PresetDto> {
    fretwire_commands::save_preset(&state, bank, slot, name).await
}

#[tauri::command]
pub async fn rename_preset(
    state: State<'_, AppState>,
    bank: i64,
    slot: i64,
    name: String,
) -> R<()> {
    fretwire_commands::rename_preset(&state, bank, slot, name).await
}

#[tauri::command]
pub async fn rename_snapshot(state: State<'_, AppState>, index: i64, name: String) -> R<PresetDto> {
    fretwire_commands::rename_snapshot(&state, index, name).await
}

#[tauri::command]
pub async fn list_presets(state: State<'_, AppState>, bank: Option<i64>) -> R<Vec<PresetListItem>> {
    fretwire_commands::list_presets(&state, bank).await
}

#[tauri::command]
pub async fn cross_setlist_write_allowed() -> bool {
    fretwire_commands::cross_setlist_write_allowed().await
}

#[tauri::command]
pub async fn setlists(state: State<'_, AppState>) -> R<Vec<String>> {
    fretwire_commands::setlists(&state).await
}

#[tauri::command]
pub async fn export_setlists(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    banks: Vec<i64>,
) -> R<i64> {
    fretwire_commands::export_setlists(&state, TauriSink(app), path, banks).await
}

#[tauri::command]
pub async fn cancel_export(state: State<'_, AppState>) -> R<()> {
    fretwire_commands::cancel_export(&state).await
}

#[tauri::command]
pub async fn backup_show(path: String) -> R<Vec<PresetListItem>> {
    fretwire_commands::backup_show(path).await
}

#[tauri::command]
pub async fn restore_preset(
    state: State<'_, AppState>,
    path: String,
    index: i64,
    slot: i64,
    bank: i64,
) -> R<PresetDto> {
    fretwire_commands::restore_preset(&state, path, index, slot, bank).await
}

// The `_inline` variants exist for serve mode (the browser's files are not the daemon's); the
// Tauri UI takes the path routes above, but the whole surface is registered so the three
// transports expose one command set.

#[tauri::command]
pub async fn export_setlists_inline(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    banks: Vec<i64>,
) -> R<BackupFileDto> {
    fretwire_commands::export_setlists_inline(&state, TauriSink(app), banks).await
}

#[tauri::command]
pub async fn backup_show_inline(json: String) -> R<Vec<PresetListItem>> {
    fretwire_commands::backup_show_inline(json).await
}

#[tauri::command]
pub async fn restore_preset_inline(
    state: State<'_, AppState>,
    json: String,
    index: i64,
    slot: i64,
    bank: i64,
) -> R<PresetDto> {
    fretwire_commands::restore_preset_inline(&state, json, index, slot, bank).await
}

#[tauri::command]
pub async fn backup_device(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    banks: Vec<i64>,
    irs: bool,
    settings: bool,
) -> R<BackupSummaryDto> {
    fretwire_commands::backup_device(&state, TauriSink(app), path, banks, irs, settings).await
}

#[tauri::command]
pub async fn backup_device_inline(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    banks: Vec<i64>,
    irs: bool,
    settings: bool,
) -> R<BackupFileDto> {
    fretwire_commands::backup_device_inline(&state, TauriSink(app), banks, irs, settings).await
}

#[tauri::command]
pub async fn backup_info(path: String) -> R<BackupInfoDto> {
    fretwire_commands::backup_info(path).await
}

#[tauri::command]
pub async fn backup_info_inline(json: String) -> R<BackupInfoDto> {
    fretwire_commands::backup_info_inline(json).await
}

#[tauri::command]
pub async fn restore_device(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    presets: bool,
    irs: bool,
    settings: bool,
) -> R<RestoreReportDto> {
    fretwire_commands::restore_device(&state, TauriSink(app), path, presets, irs, settings).await
}

#[tauri::command]
pub async fn restore_device_inline(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    json: String,
    presets: bool,
    irs: bool,
    settings: bool,
) -> R<RestoreReportDto> {
    fretwire_commands::restore_device_inline(&state, TauriSink(app), json, presets, irs, settings)
        .await
}

// ---- clipboards ----

#[tauri::command]
pub async fn copy_preset(state: State<'_, AppState>) -> R<String> {
    fretwire_commands::copy_preset(&state).await
}

#[tauri::command]
pub async fn paste_preset(state: State<'_, AppState>) -> R<PresetDto> {
    fretwire_commands::paste_preset(&state).await
}

#[tauri::command]
pub fn clipboard_preset(state: State<AppState>) -> Option<String> {
    fretwire_commands::clipboard_preset(&state)
}

#[tauri::command]
pub async fn copy_block(state: State<'_, AppState>, slot: i64) -> R<String> {
    fretwire_commands::copy_block(&state, slot).await
}

#[tauri::command]
pub async fn paste_block(state: State<'_, AppState>, slot: i64) -> R<PresetDto> {
    fretwire_commands::paste_block(&state, slot).await
}

#[tauri::command]
pub fn clipboard_block(state: State<AppState>) -> Option<String> {
    fretwire_commands::clipboard_block(&state)
}

// ---- model picker (catalog, no device mutation) ----

#[tauri::command]
pub fn split_types() -> Vec<SplitTypeDto> {
    fretwire_commands::split_types()
}

#[tauri::command]
pub async fn categories(state: State<'_, AppState>) -> R<Vec<CategoryDto>> {
    fretwire_commands::categories(&state).await
}

#[tauri::command]
pub async fn models_in_category(
    state: State<'_, AppState>,
    category: i64,
    variant: Option<String>,
) -> R<Vec<ModelChoiceDto>> {
    fretwire_commands::models_in_category(&state, category, variant).await
}

#[tauri::command]
pub async fn device_numbering(state: State<'_, AppState>) -> R<Option<String>> {
    fretwire_commands::device_numbering(&state).await
}

// ---- device settings (globals) ----

#[tauri::command]
pub async fn settings_read(state: State<'_, AppState>, all: bool) -> R<Vec<SettingDto>> {
    fretwire_commands::settings_read(&state, all).await
}

#[tauri::command]
pub async fn settings_write(state: State<'_, AppState>, id: i64, value: f64) -> R<SettingDto> {
    fretwire_commands::settings_write(&state, id, value).await
}

// ---- user IR slots ----

#[tauri::command]
pub async fn ir_list(state: State<'_, AppState>) -> R<Vec<IrSlotDto>> {
    fretwire_commands::ir_list(&state).await
}

#[tauri::command]
pub async fn ir_scan(state: State<'_, AppState>) -> R<Vec<IrSlotDto>> {
    fretwire_commands::ir_scan(&state).await
}

#[tauri::command]
pub async fn ir_export(state: State<'_, AppState>, slot: i64, path: String) -> R<String> {
    fretwire_commands::ir_export(&state, slot, path).await
}

#[tauri::command]
pub async fn ir_upload(
    state: State<'_, AppState>,
    slot: i64,
    path: String,
    name: Option<String>,
    overwrite: bool,
    force: bool,
) -> R<Vec<IrSlotDto>> {
    fretwire_commands::ir_upload(&state, slot, path, name, overwrite, force).await
}

#[tauri::command]
pub async fn ir_delete(state: State<'_, AppState>, slot: i64) -> R<Vec<IrSlotDto>> {
    fretwire_commands::ir_delete(&state, slot).await
}

#[tauri::command]
pub async fn ir_rename(state: State<'_, AppState>, slot: i64, name: String) -> R<Vec<IrSlotDto>> {
    fretwire_commands::ir_rename(&state, slot, name).await
}

#[tauri::command]
pub async fn ir_export_inline(state: State<'_, AppState>, slot: i64) -> R<IrFileDto> {
    fretwire_commands::ir_export_inline(&state, slot).await
}

#[tauri::command]
pub async fn ir_upload_inline(
    state: State<'_, AppState>,
    slot: i64,
    wav_base64: String,
    name: String,
    overwrite: bool,
    force: bool,
) -> R<Vec<IrSlotDto>> {
    fretwire_commands::ir_upload_inline(&state, slot, wav_base64, name, overwrite, force).await
}
