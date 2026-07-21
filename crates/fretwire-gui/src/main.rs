//! fretwire — graphical HX Stomp editor for Linux (iced), built on `fretwire-core`.
//!
//! A live HX Stomp editor over a held-open session: **Connect & Pull** opens a session (kept open),
//! a preset sidebar browses/switches presets, the **signal chain** (a custom canvas) draws the
//! blocks as the real serial/parallel path, and the selected block's params edit live via sliders.
//! All device I/O runs on `spawn_blocking` against a shared `Session`, so the UI never freezes and
//! ops stay serialized. An optional `dump-raw` file arg gives an offline preview (read-only).

mod chain;

use fretwire_core::{Catalog, EditorBlock, EditorParam, EditorPreset, Session};
use fretwire_data::stream::{ParamValue, StatusPush};
use iced::widget::{
    button, checkbox, column, container, pick_list, progress_bar, row, scrollable, slider, text,
    text_input,
};
use iced::{Element, Length, Task};
use std::future::Future;
use std::sync::{Arc, Mutex};

/// The live session, shared between the UI and the background workers. `None` until connected.
type Shared = Arc<Mutex<Option<Session>>>;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    iced::application("fretwire", App::update, App::view)
        .subscription(App::subscription)
        .run_with(App::new)
}

#[derive(Debug, Clone)]
enum Message {
    Connect,
    Connected(Result<(EditorPreset, Vec<(u16, String)>), String>),
    Disconnect,
    Disconnected,
    /// Navigate to a preset by its device index (loads it into the edit buffer).
    Goto(u16),
    /// A `Goto` finished: the index we navigated to + the freshly-read preset (or an error).
    Navigated(u16, Result<EditorPreset, String>),
    /// Toggle a block's bypass: (slot, new bypassed state).
    ToggleBypass(i64, bool),
    /// Slider drag: update the UI value only (slot, paired?, param index, new value). No device I/O —
    /// we commit on release so a drag doesn't flood the edit channel. `paired` = the block's cab/IR.
    ParamChanged(i64, bool, usize, f32),
    /// Slider released: send the param's current value to the device (slot, paired?, param index).
    ParamCommitted(i64, bool, usize),
    /// Enum dropdown selection: send an integer-valued param (slot, paired?, param index, option
    /// index). Discrete, so it commits immediately (no drag phase).
    ParamEnumSelected(i64, bool, usize, i64),
    /// A block was clicked in the signal chain — focus its params below.
    SelectBlock(i64),
    /// Mouse pressed on a chain block (slot): begin a potential drag/click.
    ChipPressed(i64),
    /// Cursor entered an insertion gap `(row, pos)` (row 0 = series/A, 1 = parallel/B; pos = blocks
    /// before it) — while dragging, the drop target. Emitted by the wires and by chips.
    GapEntered(u8, usize),
    /// Cursor moved (to this point) during a drag — promotes the press into a real drag once it has
    /// moved past a small threshold (so a click's pixel jitter doesn't count), revealing row B.
    DragMoved(iced::Point),
    /// Mouse released over the chain: drop (reorder) if a gap was targeted, else select.
    ChipReleased,
    /// Reorder the block at `src_slot` into gap `gap` (issued on a drag drop).
    ReorderTo(i64, usize),
    /// A block move (reorder) finished; carries the re-read preset (or an error).
    Moved(Result<EditorPreset, String>),
    /// Move the block at `slot` to the parallel (B) row (`true`) or back to series (A) (`false`), at
    /// insertion index `pos` among the target row's blocks (`usize::MAX` = append at the end).
    MoveToRow(i64, bool, usize),
    /// Change the split node at `slot` to split-type model `index` (Helix.sym); then re-read.
    SetSplitType(i64, i64),
    /// Move the block at `slot` into the common (pre-split) section, just before the split.
    DropBeforeSplit(i64),
    /// Open/close the add-block picker (append a new block to the chain).
    ToggleAddPicker,
    /// Add the chosen model to the end of the chain (then re-read); the user can drag it into place.
    AddBlockModel(i64),
    /// Open/close the model picker for the selected block.
    ToggleModelPicker,
    /// Browse a different category in the open model picker.
    SetPickerCategory(i64),
    /// Swap the block in `slot` to model `index` (Helix.sym), preserving paired cab `paired`.
    SwapModel(i64, i64, i64),
    /// A swap finished and the preset was re-read (or an error).
    Swapped(Result<EditorPreset, String>),
    /// Switch the active snapshot (0-based index).
    SetSnapshot(i64),
    /// A snapshot switch to `index` finished and the preset was re-read (or an error).
    SnapshotSet(i64, Result<EditorPreset, String>),
    /// Re-read the current preset from the device (sync after panel-side changes).
    Refresh,
    Refreshed(Result<EditorPreset, String>),
    /// Start/cancel "Save As" (save the edit buffer to a chosen slot under a new name).
    ToggleSaveAs,
    /// Edit the Save-As name.
    SaveAsNameEdited(String),
    /// Pick the target slot for Save As (from the sidebar): index + its current name.
    SaveAsPickSlot(u16, String),
    /// Commit Save As: save the edit buffer to the chosen slot under the new name (op 71, flash).
    SaveAsCommit,
    /// A Save As finished: (target index, name, result).
    SavedAs(u16, String, Result<(), String>),
    /// Delete the block at `slot` (op 28 surgical; keeps the footswitch layout).
    DeleteBlock(i64),
    /// Begin a name-only rename of the current preset (open the inline field, pre-filled).
    RenameStart,
    /// Edit the in-progress rename text.
    RenameEdited(String),
    /// Cancel the in-progress rename.
    RenameCancel,
    /// Commit the rename (op 6, name-only — does NOT save the edit buffer).
    RenameCommit,
    /// A rename finished: (preset index, new name, result).
    Renamed(u16, String, Result<(), String>),
    /// Arm/disarm the persistent save (the overwrite-to-flash confirm).
    ToggleSaveArm,
    /// Commit the save: overwrite the current preset slot in flash.
    SaveToDevice,
    Saved(Result<(), String>),
    Edited(Result<(), String>),
    /// Heartbeat: time to service the open session (keepalive + collect device state-pushes).
    Tick,
    /// Heartbeat result: the device's unsolicited state-pushes since the last beat.
    Polled(Result<Vec<StatusPush>, String>),
}

struct App {
    device: Shared,
    /// The bundled reference catalog — used to enumerate swap candidates (works offline too, and
    /// without locking the live session).
    catalog: Catalog,
    preset: Option<EditorPreset>,
    /// Every preset on the device: (device index, name). Populated at connect.
    presets: Vec<(u16, String)>,
    /// The preset index currently loaded, once we've navigated to one this session.
    current_preset: Option<u16>,
    /// The active snapshot index. Tracked in app state because the preset stream's stored
    /// `active_snapshot` lags a live switch — we trust the index we switched to.
    current_snapshot: Option<i64>,
    /// The block whose params are shown in the panel below the chain.
    selected_slot: Option<i64>,
    /// Whether the model picker (swap list) is open for the selected block.
    show_picker: bool,
    /// Which category the picker is browsing (defaults to the selected block's, but the user can
    /// switch to swap into a different category).
    picker_category: Option<i64>,
    /// Two-click guard for the persistent save (overwrites the preset in flash). `true` = armed.
    save_armed: bool,
    /// Whether the add-block picker is open (choose a model to append to the chain).
    adding: bool,
    /// "Save As" state: the new name + chosen target slot. `None` when not saving-as.
    save_as: Option<SaveAs>,
    /// In-progress name-only rename of the current preset (the editable text). `None` when idle.
    rename_buf: Option<String>,
    status: String,
    /// True while a connect/disconnect is in flight (gates the action button).
    busy: bool,
    /// True while a preset navigation is in flight (gates the preset list).
    navigating: bool,
    connected: bool,
    /// True while a heartbeat keepalive is in flight, so ticks don't pile up.
    ticking: bool,
    /// In-progress block drag (chain reorder): the pressed block and the drop target under the
    /// cursor. `None` when not dragging. A press starts it; entering another chip sets the target;
    /// release either moves (if a target differs) or selects (a plain click).
    drag: Option<Drag>,
}

/// "Save As" state: the name to write and the chosen target slot (picked from the sidebar). The
/// overwrite confirmation appears once a target is chosen.
#[derive(Debug, Clone, Default)]
struct SaveAs {
    name: String,
    /// Target preset index + its current name (for the overwrite confirm). `None` until picked.
    target: Option<(u16, String)>,
}

/// State of a chain drag-to-reorder gesture.
#[derive(Debug, Clone, Copy)]
struct Drag {
    /// Slot of the block being dragged.
    src_slot: i64,
    /// Insertion gap under the cursor as `(row, pos)`. `None` until the cursor moves over a gap — its
    /// presence at release is what distinguishes a drag (reorder/move) from a click (select).
    target_gap: Option<(u8, usize)>,
    /// `true` once the cursor has moved past the drag threshold since the press — gates revealing the
    /// parallel (B) row, so a plain click (with its pixel jitter) doesn't flash it.
    started: bool,
    /// The first cursor point seen after the press, to measure drag distance against.
    anchor: Option<iced::Point>,
}

impl App {
    fn new() -> (App, Task<Message>) {
        // Optional offline preview from a `dump-raw` file arg (read-only — not connected).
        let (preset, status) = match std::env::args().nth(1) {
            Some(path) => match load(&path) {
                Ok(p) => (Some(p), format!("loaded {path} (offline preview)")),
                Err(e) => (None, format!("failed to load {path}: {e}")),
            },
            None => (None, "Connect to the pedal, or pass a dump-raw .bin file.".to_string()),
        };
        let app = App {
            device: Arc::new(Mutex::new(None)),
            catalog: Catalog::load().expect("load reference data (bundled or imported)"),
            preset,
            presets: Vec::new(),
            current_preset: None,
            current_snapshot: None,
            selected_slot: None,
            show_picker: false,
            picker_category: None,
            save_armed: false,
            adding: false,
            save_as: None,
            rename_buf: None,
            status,
            busy: false,
            navigating: false,
            connected: false,
            ticking: false,
            drag: None,
        };
        (app, Task::none())
    }

    /// Heartbeat while connected so the device keeps servicing the session (see `Session::keepalive`).
    fn subscription(&self) -> iced::Subscription<Message> {
        if self.connected {
            iced::time::every(std::time::Duration::from_millis(250)).map(|_| Message::Tick)
        } else {
            iced::Subscription::none()
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Connect => {
                if self.busy || self.connected {
                    return Task::none();
                }
                self.busy = true;
                self.status = "connecting to the pedal…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || {
                        let mut s = Session::connect().map_err(|e| e.to_string())?;
                        let preset = s.read_preset().map_err(|e| e.to_string())?;
                        // Pull the preset list too, so the sidebar is ready to navigate.
                        let presets = s.list_presets().map_err(|e| e.to_string())?;
                        *dev.lock().unwrap() = Some(s); // keep the session open for live edits
                        Ok((preset, presets))
                    }),
                    Message::Connected,
                )
            }
            Message::Connected(Ok((preset, presets))) => {
                self.current_preset = preset.current.as_ref().map(|i| i.index as u16);
                let where_ = match &preset.current {
                    Some(i) => format!("preset {} {}", i.index, i.name),
                    None => "preset unknown".to_string(),
                };
                self.status = format!(
                    "connected — on {where_} · {} block(s), DSP {:.1}% · {} presets",
                    preset.blocks.len(),
                    preset.dsp_load,
                    presets.len(),
                );
                self.current_snapshot = preset.active_snapshot;
                self.selected_slot = first_block_slot(&preset);
                self.preset = Some(preset);
                self.presets = presets;
                self.connected = true;
                self.busy = false;
                self.save_armed = false;
                Task::none()
            }
            Message::Connected(Err(e)) => {
                self.status = format!("connect failed: {e}");
                self.busy = false;
                Task::none()
            }
            Message::Disconnect => {
                if self.busy || !self.connected {
                    return Task::none();
                }
                self.busy = true;
                self.status = "disconnecting…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || {
                        // Dropping the session runs its teardown (releases the panel lock).
                        let session = dev.lock().unwrap().take();
                        drop(session);
                        Ok(())
                    }),
                    |_| Message::Disconnected,
                )
            }
            Message::Disconnected => {
                self.connected = false;
                self.busy = false;
                self.presets.clear();
                self.current_preset = None;
                self.save_armed = false;
                self.status = "disconnected — pedal back to standalone".to_string();
                Task::none()
            }
            Message::Goto(index) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true;
                self.status = format!("loading preset {index}…");
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => {
                            s.goto_preset(0, index as i64).map_err(|e| e.to_string())?;
                            s.read_preset().map_err(|e| e.to_string())
                        }
                        None => Err("not connected".to_string()),
                    }),
                    move |r| Message::Navigated(index, r),
                )
            }
            Message::Navigated(index, Ok(preset)) => {
                self.navigating = false;
                // Trust the device's own read-info reply for which preset is loaded; fall back to
                // the index we asked for if it's missing.
                let (cur, name) = match &preset.current {
                    Some(i) => (i.index as u16, i.name.clone()),
                    None => (
                        index,
                        self.presets
                            .iter()
                            .find(|(i, _)| *i == index)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| "?".to_string()),
                    ),
                };
                self.current_preset = Some(cur);
                self.status = format!(
                    "preset {cur} {name} — {} block(s), DSP {:.1}%",
                    preset.blocks.len(),
                    preset.dsp_load,
                );
                self.current_snapshot = preset.active_snapshot;
                self.selected_slot = first_block_slot(&preset);
                self.preset = Some(preset);
                self.save_armed = false;
                Task::none()
            }
            Message::Navigated(_, Err(e)) => {
                self.navigating = false;
                self.status = format!("preset load failed: {e}");
                Task::none()
            }
            Message::ToggleBypass(slot, bypassed) => {
                // Optimistic UI update; the device edit confirms (or reports an error) async.
                if let Some(p) = &mut self.preset {
                    if let Some(b) = p.blocks.iter_mut().find(|b| b.slot == slot) {
                        b.bypassed = Some(bypassed);
                    }
                }
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.set_enabled(slot, !bypassed).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Edited,
                )
            }
            Message::ParamChanged(slot, paired, index, value) => {
                // UI-only: reflect the drag immediately; the device edit waits for release.
                if let Some(p) = &mut self.preset {
                    if let Some(b) = p.block_mut(slot) {
                        let params = if paired { &mut b.paired_params } else { &mut b.params };
                        if let Some(prm) = params.get_mut(index) {
                            prm.value = ParamValue::Float(value);
                        }
                    }
                }
                Task::none()
            }
            Message::ParamCommitted(slot, paired, index) => {
                // Send the value the drag left in the UI model.
                let value = self.preset.as_ref().and_then(|p| {
                    let b = p.block(slot)?;
                    let params = if paired { &b.paired_params } else { &b.params };
                    params.get(index).map(|prm| fmt_f32(prm.value))
                });
                let Some(value) = value else { return Task::none() };
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => {
                            let r = if paired {
                                s.set_paired_param(slot, index as i64, value)
                            } else {
                                s.set_param(slot, index as i64, value)
                            };
                            r.map_err(|e| e.to_string())
                        }
                        None => Err("not connected".to_string()),
                    }),
                    Message::Edited,
                )
            }
            Message::ParamEnumSelected(slot, paired, index, value) => {
                // Reflect immediately, then send the enum index to the device.
                if let Some(p) = &mut self.preset {
                    if let Some(b) = p.block_mut(slot) {
                        let params = if paired { &mut b.paired_params } else { &mut b.params };
                        if let Some(prm) = params.get_mut(index) {
                            prm.value = ParamValue::Int(value);
                        }
                    }
                }
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s
                            .set_param_enum(slot, paired, index as i64, value)
                            .map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Edited,
                )
            }
            Message::SelectBlock(slot) => {
                if self.selected_slot != Some(slot) {
                    self.show_picker = false; // a different block → close the stale picker
                }
                self.selected_slot = Some(slot);
                Task::none()
            }
            Message::ChipPressed(slot) => {
                // Begin a press: a release without moving over a gap is a click (select); moving
                // over a gap turns it into a reorder.
                self.drag =
                    Some(Drag { src_slot: slot, target_gap: None, started: false, anchor: None });
                Task::none()
            }
            Message::GapEntered(row, pos) => {
                if let Some(d) = &mut self.drag {
                    d.target_gap = Some((row, pos));
                    // NOTE: do NOT mark `started` here — on_enter re-fires on the post-press
                    // re-render (cursor still over the source chip), which would flash row B on a
                    // plain click. Only real cursor movement (DragMoved) promotes a press to a drag.
                }
                Task::none()
            }
            Message::DragMoved(p) => {
                // Require movement past a small threshold so a click's pixel jitter isn't a "drag".
                const DRAG_THRESHOLD: f32 = 8.0;
                if let Some(d) = &mut self.drag {
                    match d.anchor {
                        None => d.anchor = Some(p),
                        Some(a) => {
                            if a.distance(p) > DRAG_THRESHOLD {
                                d.started = true;
                            }
                        }
                    }
                }
                Task::none()
            }
            Message::ChipReleased => {
                let Some(d) = self.drag.take() else { return Task::none() };
                let src_row = self.block_row(d.src_slot);
                let split = self.preset.as_ref().is_some_and(|p| p.split);
                let ready = self.connected && !self.busy && !self.navigating;
                match d.target_gap {
                    // Sentinel row 2 = the drop zone just before the split chip (common-before end).
                    Some((2, _)) if ready && split => {
                        self.update(Message::DropBeforeSplit(d.src_slot))
                    }
                    Some((row, pos)) if ready && row != src_row => {
                        // Dropped on the other row at position `pos` → move there (the landing column
                        // sets the split/mixer; creates/retires the split as needed).
                        self.update(Message::MoveToRow(d.src_slot, row == 1, pos))
                    }
                    // Same-row reorder (row == src_row here). A gap flanking the block's own spot is a
                    // no-op → just select. Removing the block shifts later positions down by one, so a
                    // gap past it maps one index earlier.
                    Some((row, pos)) if ready => {
                        let from = self.row_pos(d.src_slot, row);
                        if from.is_some_and(|f| pos == f || pos == f + 1) {
                            self.update(Message::SelectBlock(d.src_slot))
                        } else {
                            let adj = from.map_or(pos, |f| if pos > f { pos - 1 } else { pos });
                            if split {
                                // Reorder within a parallel row (A or B) — same path as a cross-row move.
                                self.update(Message::MoveToRow(d.src_slot, row == 1, adj))
                            } else {
                                self.update(Message::ReorderTo(d.src_slot, pos))
                            }
                        }
                    }
                    _ => self.update(Message::SelectBlock(d.src_slot)),
                }
            }
            Message::ReorderTo(src_slot, gap) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true;
                self.show_picker = false;
                self.status = "reordering…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.reorder_block(src_slot, gap).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Moved,
                )
            }
            Message::Moved(Ok(preset)) => {
                self.navigating = false;
                // Keep the current selection if that slot still holds a block, else fall back.
                // Node-aware: the split/mixer nodes live outside `blocks`, so keep them selected too.
                if !self.selected_slot.is_some_and(|s| preset.block(s).is_some()) {
                    self.selected_slot = first_block_slot(&preset);
                }
                self.status = format!("block moved · DSP {:.1}%", preset.dsp_load);
                self.preset = Some(preset);
                Task::none()
            }
            Message::Moved(Err(e)) => {
                self.navigating = false;
                self.status = format!("reorder failed: {e} — ⟳ Refresh");
                Task::none()
            }
            Message::MoveToRow(slot, parallel, pos) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true;
                self.show_picker = false;
                self.status =
                    format!("moving block to {} row…", if parallel { "parallel" } else { "series" });
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.move_block_to_row(slot, parallel, pos).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Moved,
                )
            }
            Message::DropBeforeSplit(slot) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true;
                self.show_picker = false;
                self.status = "moving block before the split…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.move_before_split(slot).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Moved,
                )
            }
            Message::SetSplitType(slot, index) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true;
                self.show_picker = false;
                self.status = "changing split type…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.set_split_type(slot, index).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Moved, // structural edit done → re-read (same as reorder/move)
                )
            }
            Message::ToggleAddPicker => {
                self.adding = !self.adding;
                self.show_picker = false;
                if self.adding && self.picker_category.is_none() {
                    // Default to the first listed category so the model list isn't empty.
                    self.picker_category = self.catalog.categories().first().map(|(id, _)| *id);
                }
                Task::none()
            }
            Message::AddBlockModel(model_index) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.adding = false;
                self.navigating = true;
                self.status = "adding block…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.add_block_append(model_index, -1).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Moved, // structural edit done → re-read (same as reorder)
                )
            }
            Message::ToggleModelPicker => {
                self.adding = false;
                self.show_picker = !self.show_picker;
                if self.show_picker {
                    // Open on the selected block's own category.
                    self.picker_category = self
                        .selected_slot
                        .and_then(|slot| self.preset.as_ref()?.blocks.iter().find(|b| b.slot == slot))
                        .and_then(|b| b.category);
                }
                Task::none()
            }
            Message::SetPickerCategory(cat) => {
                self.picker_category = Some(cat);
                Task::none()
            }
            Message::SwapModel(slot, index, paired) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true; // reuse the navigation gate (a swap re-reads the preset)
                self.show_picker = false;
                self.status = "swapping model…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => {
                            s.swap_model(slot, index, paired).map_err(|e| e.to_string())?;
                            s.read_preset().map_err(|e| e.to_string())
                        }
                        None => Err("not connected".to_string()),
                    }),
                    Message::Swapped,
                )
            }
            Message::Swapped(Ok(preset)) => {
                self.navigating = false;
                self.status = format!(
                    "model swapped — {} block(s), DSP {:.1}%",
                    preset.blocks.len(),
                    preset.dsp_load,
                );
                self.preset = Some(preset); // selection (slot) is unchanged; the block is new there
                Task::none()
            }
            Message::Swapped(Err(e)) => {
                self.navigating = false;
                self.status = format!("swap failed: {e}");
                Task::none()
            }
            Message::SetSnapshot(index) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true;
                self.status = format!("switching to snapshot {}…", index + 1);
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => {
                            s.set_snapshot(index).map_err(|e| e.to_string())?;
                            s.read_preset().map_err(|e| e.to_string())
                        }
                        None => Err("not connected".to_string()),
                    }),
                    move |r| Message::SnapshotSet(index, r),
                )
            }
            Message::SnapshotSet(index, Ok(preset)) => {
                self.navigating = false;
                // Trust the index we switched to (the stored active_snapshot lags).
                self.current_snapshot = Some(index);
                self.status = format!("snapshot {} · DSP {:.1}%", index + 1, preset.dsp_load);
                self.preset = Some(preset);
                Task::none()
            }
            Message::SnapshotSet(index, Err(e)) => {
                self.navigating = false;
                // The snapshot was sent before the re-read; the device switched even if the
                // read-back failed — reflect the selection and suggest a manual refresh.
                self.current_snapshot = Some(index);
                self.status = format!("snapshot {} set; view re-read failed ({e}) — ⟳ Refresh", index + 1);
                Task::none()
            }
            Message::Refresh => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true;
                self.status = "refreshing from device…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.read_preset().map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Refreshed,
                )
            }
            Message::Refreshed(Ok(preset)) => {
                self.navigating = false;
                // Note: don't reset current_snapshot from preset.active_snapshot — the stored field
                // lags a live switch, and a panel-driven snapshot/preset push already set the right
                // value before triggering this refresh.
                // Keep the current selection if that slot still exists, else fall back.
                // Node-aware: the split/mixer nodes live outside `blocks`, so keep them selected too.
                if !self.selected_slot.is_some_and(|s| preset.block(s).is_some()) {
                    self.selected_slot = first_block_slot(&preset);
                }
                self.status = format!("refreshed · DSP {:.1}%", preset.dsp_load);
                self.preset = Some(preset);
                Task::none()
            }
            Message::Refreshed(Err(e)) => {
                self.navigating = false;
                self.status = format!("refresh failed: {e}");
                Task::none()
            }
            Message::ToggleSaveAs => {
                self.save_as = match self.save_as {
                    Some(_) => None, // cancel
                    None => {
                        let name = self
                            .preset
                            .as_ref()
                            .and_then(|p| p.current.as_ref())
                            .map(|c| c.name.clone())
                            .unwrap_or_default();
                        self.save_armed = false;
                        Some(SaveAs { name, target: None })
                    }
                };
                Task::none()
            }
            Message::SaveAsNameEdited(s) => {
                if let Some(sa) = &mut self.save_as {
                    sa.name = s;
                }
                Task::none()
            }
            Message::SaveAsPickSlot(idx, name) => {
                if let Some(sa) = &mut self.save_as {
                    sa.target = Some((idx, name));
                }
                Task::none()
            }
            Message::SaveAsCommit => {
                let Some(sa) = self.save_as.clone() else { return Task::none() };
                let (Some((idx, _)), name) = (sa.target, sa.name.trim().to_string()) else {
                    return Task::none();
                };
                if name.is_empty() || self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.save_as = None;
                self.navigating = true;
                self.status = format!("saving to preset {idx} as {name:?}…");
                let dev = self.device.clone();
                let n = name.clone();
                // Bank 0 (the user setlist), matching how presets are listed/navigated.
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.save_preset(0, idx as i64, &n).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    move |r| Message::SavedAs(idx, name.clone(), r),
                )
            }
            Message::SavedAs(idx, name, result) => {
                self.navigating = false;
                match result {
                    Ok(()) => {
                        // Reflect the new name in the sidebar; if we overwrote the loaded slot, update
                        // its identity too.
                        if let Some(e) = self.presets.iter_mut().find(|(i, _)| *i == idx) {
                            e.1 = name.clone();
                        }
                        if self.current_preset == Some(idx) {
                            if let Some(cur) = self.preset.as_mut().and_then(|p| p.current.as_mut()) {
                                cur.name = name.clone();
                            }
                        }
                        self.status = format!("saved to preset {idx} as {name:?}");
                    }
                    Err(e) => self.status = format!("save-as failed: {e}"),
                }
                Task::none()
            }
            Message::DeleteBlock(slot) => {
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.navigating = true;
                self.show_picker = false;
                self.status = "deleting block…".to_string();
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.delete_block(slot).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Moved, // structural edit done → re-read (same as reorder/add)
                )
            }
            Message::RenameStart => {
                // Pre-fill with the current preset's name; HX Edit renames without a confirm step.
                self.rename_buf = Some(
                    self.preset
                        .as_ref()
                        .and_then(|p| p.current.as_ref())
                        .map(|c| c.name.clone())
                        .unwrap_or_default(),
                );
                Task::none()
            }
            Message::RenameEdited(s) => {
                if let Some(buf) = &mut self.rename_buf {
                    *buf = s;
                }
                Task::none()
            }
            Message::RenameCancel => {
                self.rename_buf = None;
                Task::none()
            }
            Message::RenameCommit => {
                let Some(name) = self.rename_buf.clone().map(|s| s.trim().to_string()) else {
                    return Task::none();
                };
                let Some(cur) = self.preset.as_ref().and_then(|p| p.current.clone()) else {
                    self.status = "can't rename — current preset identity unknown".to_string();
                    return Task::none();
                };
                if name.is_empty() || self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                self.rename_buf = None;
                self.status = format!("renaming to {name:?}…");
                let dev = self.device.clone();
                let (bank, idx) = (cur.bank, cur.index);
                let n = name.clone();
                // Name-only (op 6): does NOT commit the edit buffer, so no re-read is needed.
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.rename_preset(bank, idx, &n).map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    move |r| Message::Renamed(idx as u16, name.clone(), r),
                )
            }
            Message::Renamed(idx, name, result) => {
                match result {
                    Ok(()) => {
                        // Reflect the new name in the sidebar and the current preset identity.
                        if let Some(e) = self.presets.iter_mut().find(|(i, _)| *i == idx) {
                            e.1 = name.clone();
                        }
                        if let Some(cur) = self.preset.as_mut().and_then(|p| p.current.as_mut()) {
                            cur.name = name.clone();
                        }
                        self.status = format!("renamed to {name:?}");
                    }
                    Err(e) => self.status = format!("rename failed: {e}"),
                }
                Task::none()
            }
            Message::ToggleSaveArm => {
                self.save_armed = !self.save_armed;
                Task::none()
            }
            Message::SaveToDevice => {
                self.save_armed = false;
                if self.busy || self.navigating || !self.connected {
                    return Task::none();
                }
                // Overwrite the current preset slot in flash, keeping its bank/slot/name.
                let Some(cur) = self.preset.as_ref().and_then(|p| p.current.clone()) else {
                    self.status = "can't save — current preset identity unknown".to_string();
                    return Task::none();
                };
                self.navigating = true;
                self.status = format!("saving to preset {} {}…", cur.index, cur.name);
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s
                            .save_preset(cur.bank, cur.index, &cur.name)
                            .map_err(|e| e.to_string()),
                        None => Err("not connected".to_string()),
                    }),
                    Message::Saved,
                )
            }
            Message::Saved(result) => {
                self.navigating = false;
                self.status = match result {
                    Ok(()) => "saved to device".to_string(),
                    Err(e) => format!("save failed: {e}"),
                };
                Task::none()
            }
            Message::Edited(Ok(())) => Task::none(),
            Message::Edited(Err(e)) => {
                self.status = format!("edit failed: {e}");
                Task::none()
            }
            Message::Tick => {
                // Skip if not connected, mid connect/disconnect/navigation, or a prior heartbeat
                // is still going.
                if !self.connected || self.busy || self.navigating || self.ticking {
                    return Task::none();
                }
                self.ticking = true;
                let dev = self.device.clone();
                Task::perform(
                    blocking(move || match dev.lock().unwrap().as_mut() {
                        Some(s) => s.poll_events().map_err(|e| e.to_string()),
                        None => Ok(Vec::new()),
                    }),
                    Message::Polled,
                )
            }
            Message::Polled(result) => {
                self.ticking = false;
                let pushes = match result {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::debug!("heartbeat poll failed: {e}");
                        return Task::none();
                    }
                };
                // Apply the device's panel-side changes so the GUI follows the hardware live.
                let mut needs_refresh = false;
                for push in pushes {
                    match push {
                        StatusPush::Bypass { slot, enabled } => {
                            if let Some(p) = &mut self.preset {
                                if let Some(b) = p.blocks.iter_mut().find(|b| b.slot == slot) {
                                    b.bypassed = Some(!enabled);
                                }
                            }
                        }
                        StatusPush::Snapshot(i) => {
                            self.current_snapshot = Some(i);
                            needs_refresh = true; // params/bypass change with the snapshot
                        }
                        StatusPush::Preset(i) => {
                            self.current_preset = Some(i as u16);
                            needs_refresh = true; // the whole preset changed
                        }
                        StatusPush::Other(_) => {}
                    }
                }
                // A snapshot/preset change rewrites the whole block tree — re-read to catch up.
                if needs_refresh && !self.navigating {
                    Task::done(Message::Refresh)
                } else {
                    Task::none()
                }
            }
        }
    }

    /// The signal-path row (0 = series/A, 1 = parallel/B) of the block at `slot`; 0 if unknown.
    fn block_row(&self, slot: i64) -> u8 {
        self.preset
            .as_ref()
            .and_then(|p| p.blocks.iter().find(|b| b.slot == slot))
            .map_or(0, |b| b.row)
    }

    /// The order position of `slot` among its own row's blocks (`row` = 0 top / 1 bottom), left to
    /// right — lines up with the drop-gap indices for that row.
    fn row_pos(&self, slot: i64, row: u8) -> Option<usize> {
        let p = self.preset.as_ref()?;
        let mut order: Vec<i64> =
            p.blocks.iter().filter(|b| b.row == row && !b.is_controller).map(|b| b.slot).collect();
        order.sort_unstable();
        order.iter().position(|&s| s == slot)
    }

    fn view(&self) -> Element<'_, Message> {
        let action = if self.busy {
            button(text(if self.connected { "Disconnecting…" } else { "Connecting…" }))
        } else if self.connected {
            button(text("Disconnect")).on_press(Message::Disconnect)
        } else {
            button(text("Connect & Pull")).on_press(Message::Connect)
        };

        let header = column![
            row![text("fretwire").size(24), action].spacing(12),
            text(self.status.clone()).size(14),
        ]
        .spacing(10)
        .padding(16);

        // Right pane: device info + DSP meter (shrink) on top, the signal-chain canvas (fixed
        // height, pans horizontally), and the selected block's param panel filling the rest
        // (scrolls vertically). Only the param panel scrolls on the vertical axis — no nested
        // same-axis scrollables, which previously collapsed the chain's height.
        let content: Element<Message> = if let Some(p) = &self.preset {
            let topo = if p.split { "split (parallel)" } else { "serial" };
            let info = column![
                text(format!(
                    "{} — fw {} · {} · {} block(s)",
                    p.device_model.as_deref().unwrap_or("?"),
                    p.firmware.as_deref().unwrap_or("?"),
                    topo,
                    p.blocks.len(),
                )),
                text(format!("DSP {:.1}% used", p.dsp_load)),
                progress_bar(0.0..=100.0, p.dsp_load as f32),
            ]
            .spacing(10);

            column![
                info,
                self.preset_actions(p),
                chain::view(
                    p,
                    self.selected_slot,
                    self.drag.map(|d| (d.src_slot, d.target_gap)),
                    self.drag.is_some_and(|d| d.started),
                ),
                self.add_bar(p),
                scrollable(self.param_panel(p)).width(Length::Fill).height(Length::Fill),
            ]
            .spacing(12)
            .padding(16)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            container(text("")).width(Length::Fill).height(Length::Fill).into()
        };

        // Body: preset sidebar (when connected) beside the block tree.
        let body: Element<Message> = if self.presets.is_empty() {
            content.into()
        } else {
            row![self.preset_sidebar(), content].spacing(8).into()
        };

        container(column![header, body].spacing(0))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// The scrollable preset list. The loaded preset is highlighted; clicking one navigates the
    /// device to it (disabled while a navigation is already in flight).
    fn preset_sidebar(&self) -> Element<'_, Message> {
        // In Save-As mode the list picks the *target* slot to overwrite instead of navigating.
        let save_as = self.save_as.as_ref();
        let header = if save_as.is_some() { "Save to which slot?" } else { "Presets" };
        let mut list = column![text(format!("{header} ({})", self.presets.len())).size(14)]
            .spacing(2)
            .padding(8);
        for (index, name) in &self.presets {
            let index = *index;
            let current = self.current_preset == Some(index);
            let chosen = save_as.and_then(|s| s.target.as_ref()).map(|(i, _)| *i) == Some(index);
            let label = text(format!("{index:>3}  {name}")).size(13);
            let mut b = button(label).width(Length::Fill).style(if chosen {
                button::danger
            } else if current {
                button::primary
            } else {
                button::text
            });
            if save_as.is_some() {
                b = b.on_press(Message::SaveAsPickSlot(index, name.clone()));
            } else if !self.navigating && !current {
                b = b.on_press(Message::Goto(index));
            }
            list = list.push(b);
        }
        container(scrollable(list)).width(Length::Fixed(260.0)).height(Length::Fill).into()
    }

    /// The panel below the chain: the selected block's bypass + params (two columns), plus its
    /// paired cab/IR params. Empty hint when nothing is selected.
    fn param_panel<'a>(&self, p: &'a EditorPreset) -> Element<'a, Message> {
        let Some(slot) = self.selected_slot else {
            return text("Select a block in the chain to edit its parameters.").size(13).into();
        };
        let Some(b) = p.block(slot) else {
            return text("…").size(13).into();
        };
        let is_split = p.is_split_node(slot);
        let is_mixer = p.is_mixer_node(slot);
        let is_node = is_split || is_mixer;

        let variant = b.variant.map(|v| format!(" · {v}")).unwrap_or_default();
        let dsp = b.dsp_load.map(|l| format!(" · {l:.1}% DSP")).unwrap_or_default();
        // Routing nodes get a friendly title (their raw model name is a flow symbol); blocks show the
        // resolved model name.
        let title = if is_split {
            "Split".to_string()
        } else if is_mixer {
            "Mixer".to_string()
        } else {
            format!("{}{}{}", b.model_name, variant, dsp)
        };
        let mut head = row![text(title).size(18)].spacing(12).align_y(iced::Alignment::Center);

        if is_split {
            // The split node's "model" is its type — a pick_list, not the generic model picker.
            let items: Vec<SplitTypeItem> = fretwire_core::editor::SPLIT_TYPES
                .iter()
                .map(|&(index, symbol, label)| SplitTypeItem { index, symbol, label })
                .collect();
            let current =
                b.symbolic_id.as_deref().and_then(|s| items.iter().find(|it| it.symbol == s).copied());
            if self.connected {
                head = head.push(text("Type:").size(14));
                head = head.push(pick_list(items, current, move |it: SplitTypeItem| {
                    Message::SetSplitType(slot, it.index)
                }));
            } else if let Some(c) = current {
                head = head.push(text(format!("Type: {}", c.label)).size(14));
            }
        } else if !b.is_controller && !is_mixer {
            let mut cb = checkbox("bypassed", b.bypassed == Some(true));
            if self.connected {
                cb = cb.on_toggle(move |checked| Message::ToggleBypass(slot, checked));
            }
            head = head.push(cb);
            // Model swap is a live action and needs a known category to list candidates.
            if self.connected && b.category.is_some() {
                let lbl = if self.show_picker { "Close" } else { "Change model ▾" };
                head = head.push(button(text(lbl).size(13)).on_press(Message::ToggleModelPicker));
            }
            // Move the block between the series (A) and parallel (B) rows (op-43, FS-safe). Moving to
            // B on a serial preset creates the split; moving the last B block back to A retires it.
            if self.connected {
                let (lbl, parallel) = if b.row == 1 {
                    ("⇡ To series", false)
                } else {
                    ("⇣ To parallel", true)
                };
                head = head.push(
                    button(text(lbl).size(13)).on_press(Message::MoveToRow(slot, parallel, usize::MAX)),
                );
            }
            // Delete the block (op 28, surgical — preserves the footswitch layout of the rest).
            if self.connected {
                head = head.push(
                    button(text("✕ Delete").size(13))
                        .style(button::danger)
                        .on_press(Message::DeleteBlock(slot)),
                );
            }
        }

        let mut panel = column![head].spacing(10);
        if self.show_picker && !b.is_controller && !is_node {
            panel = panel.push(self.model_picker(p, b));
        }
        panel = panel.push(param_grid(slot, false, &b.params, self.connected));
        if let Some(cab) = &b.paired_model_name {
            panel = panel.push(text(format!("+ cab: {cab}")).size(14));
            panel = panel.push(param_grid(slot, true, &b.paired_params, self.connected));
        }

        container(panel).width(Length::Fill).padding([12, 0]).into()
    }

    /// The preset-actions bar: snapshot switches + the persistent save (overwrite-to-flash, behind a
    /// two-click confirm). Only meaningful while connected; offline it shows nothing interactive.
    fn preset_actions<'a>(&self, p: &'a EditorPreset) -> Element<'a, Message> {
        let mut bar = row![].spacing(8).align_y(iced::Alignment::Center);

        if self.connected {
            bar = bar.push(button(text("⟳ Refresh").size(12)).on_press(Message::Refresh));
        }

        if !p.snapshot_names.is_empty() {
            bar = bar.push(text("Snapshot:").size(13));
            for (i, name) in p.snapshot_names.iter().enumerate() {
                let idx = i as i64;
                let active = self.current_snapshot == Some(idx);
                let label = if name.is_empty() { format!("{}", i + 1) } else { name.clone() };
                let mut b = button(text(label).size(12))
                    .style(if active { button::primary } else { button::secondary });
                if self.connected && !active {
                    b = b.on_press(Message::SetSnapshot(idx));
                }
                bar = bar.push(b);
            }
        }

        // Rename: change only the current preset's name (op 6, name-only — does NOT save the edit
        // buffer). No confirm step, matching HX Edit. Only when we know the current slot.
        if self.connected && p.current.is_some() && self.save_as.is_none() {
            if let Some(buf) = &self.rename_buf {
                bar = bar.push(
                    text_input("preset name", buf)
                        .on_input(Message::RenameEdited)
                        .on_submit(Message::RenameCommit)
                        .size(13)
                        .width(Length::Fixed(160.0)),
                );
                bar = bar.push(button(text("Rename").size(12)).on_press(Message::RenameCommit));
                bar = bar.push(button(text("Cancel").size(12)).on_press(Message::RenameCancel));
            } else {
                bar = bar.push(button(text("Rename…").size(12)).on_press(Message::RenameStart));
            }
        }

        // Save As: write the edit buffer to a chosen slot under a new name (op 71, flash write).
        if self.connected && self.rename_buf.is_none() {
            if let Some(sa) = &self.save_as {
                bar = bar.push(
                    text_input("new name", &sa.name)
                        .on_input(Message::SaveAsNameEdited)
                        .size(13)
                        .width(Length::Fixed(160.0)),
                );
                match &sa.target {
                    Some((idx, oldname)) => {
                        bar = bar.push(
                            button(
                                text(format!("Confirm: overwrite [{idx}] {oldname}")).size(12),
                            )
                            .style(button::danger)
                            .on_press(Message::SaveAsCommit),
                        );
                    }
                    None => {
                        bar = bar.push(text("← pick a slot in the list").size(12));
                    }
                }
                bar = bar.push(button(text("Cancel").size(12)).on_press(Message::ToggleSaveAs));
            } else {
                bar = bar.push(button(text("Save As…").size(12)).on_press(Message::ToggleSaveAs));
            }
        }

        // Save (overwrite the current preset in flash). Only when we know the slot/name.
        if self.connected && p.current.is_some() && self.save_as.is_none() {
            let cur = p.current.as_ref().unwrap();
            if self.save_armed {
                bar = bar.push(
                    button(text(format!("Confirm: overwrite [{}] {}", cur.index, cur.name)).size(12))
                        .style(button::danger)
                        .on_press(Message::SaveToDevice),
                );
                bar = bar.push(button(text("Cancel").size(12)).on_press(Message::ToggleSaveArm));
            } else {
                bar = bar.push(
                    button(text("Save to device").size(12)).on_press(Message::ToggleSaveArm),
                );
            }
        }

        bar.into()
    }

    /// The model picker for `b`: a category selector plus every model in the chosen category, with
    /// DSP cost, the current model highlighted, and any that wouldn't fit the remaining DSP budget
    /// greyed out (disabled). Clicking one swaps the block and re-reads the preset. A same-category
    /// (amp→amp) swap preserves the paired cab; a cross-category swap drops it.
    fn model_picker<'a>(&self, p: &'a EditorPreset, b: &'a EditorBlock) -> Element<'a, Message> {
        let Some(category) = self.picker_category.or(b.category) else {
            return text("(no category — can't list models)").size(13).into();
        };

        // Category selector — lets the user browse a different effect type to swap into.
        let cats: Vec<CatItem> =
            self.catalog.categories().into_iter().map(|(id, name)| CatItem { id, name }).collect();
        let selected = cats.iter().find(|c| c.id == category).copied();
        let cat_picker = pick_list(cats, selected, |c: CatItem| Message::SetPickerCategory(c.id));

        let choices = self.catalog.models_in_category(category, b.variant);
        let used_without = p.dsp_load - b.dsp_load.unwrap_or(0.0);
        // Keep the paired cab/IR only for a same-category swap; a cross-category swap drops it.
        let same_cat = Some(category) == b.category;
        let paired = if same_cat { b.paired_index.unwrap_or(-1) } else { -1 };
        let cab_load =
            if paired >= 0 { self.catalog.model_load_by_index(paired).unwrap_or(0.0) } else { 0.0 };

        let mut list = column![text(format!("{} option(s)", choices.len())).size(12)].spacing(2);
        for c in choices {
            let is_current = b.symbolic_id.as_deref() == Some(c.symbolic_id.as_str());
            let load = c.dsp_load.unwrap_or(0.0);
            let projected = used_without + load + cab_load;
            let fits = projected <= fretwire_core::editor::DSP_BUDGET;
            let cost = c.dsp_load.map(|l| format!("{l:.1}%")).unwrap_or_else(|| "?".into());
            let mark = if is_current { "● " } else { "  " };
            let label = text(format!("{mark}{:<22} {cost:>6}", c.name)).size(13);

            let mut btn = button(label).width(Length::Fill).style(if is_current {
                button::primary
            } else if fits {
                button::secondary
            } else {
                button::danger
            });
            // Don't re-swap the current model; block (disable) ones that won't fit.
            if !is_current && fits {
                btn = btn.on_press(Message::SwapModel(b.slot, c.index, paired));
            }
            list = list.push(btn);
        }

        column![
            row![text("Category:").size(13), cat_picker]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            scrollable(list).height(Length::Fixed(240.0)),
        ]
        .spacing(6)
        .into()
    }

    /// The add-block affordance below the chain: a "+ Add block" toggle, expanding into the add
    /// picker when active. Only meaningful while connected on a non-split preset (append-only for now).
    fn add_bar<'a>(&self, p: &'a EditorPreset) -> Element<'a, Message> {
        if !self.connected {
            return container(text("")).into();
        }
        if self.adding {
            return self.add_picker(p);
        }
        // Appends to the series (A) row; on a split preset it fills the first free row-A slot (or
        // row B if A is full), and the user can drag it wherever afterwards.
        button(text("+ Add block").size(13)).on_press(Message::ToggleAddPicker).into()
    }

    /// The add-block picker: a category selector + every model in it (with DSP cost, fit-greyed),
    /// not tied to any existing block. Clicking one appends it to the end of the chain (the user can
    /// then drag it into position). Mono variants are listed; DSP fit is checked against the whole
    /// preset's current load.
    fn add_picker<'a>(&self, p: &'a EditorPreset) -> Element<'a, Message> {
        let Some(category) = self.picker_category else {
            return text("(pick a category)").size(13).into();
        };
        let cats: Vec<CatItem> =
            self.catalog.categories().into_iter().map(|(id, name)| CatItem { id, name }).collect();
        let selected = cats.iter().find(|c| c.id == category).copied();
        let cat_picker = pick_list(cats, selected, |c: CatItem| Message::SetPickerCategory(c.id));

        let choices = self.catalog.models_in_category(category, None);
        let mut list = column![text(format!("{} option(s)", choices.len())).size(12)].spacing(2);
        for c in choices {
            let load = c.dsp_load.unwrap_or(0.0);
            let fits = p.dsp_load + load <= fretwire_core::editor::DSP_BUDGET;
            let cost = c.dsp_load.map(|l| format!("{l:.1}%")).unwrap_or_else(|| "?".into());
            let label = text(format!("  {:<22} {cost:>6}", c.name)).size(13);
            let mut btn = button(label)
                .width(Length::Fill)
                .style(if fits { button::secondary } else { button::danger });
            if fits {
                btn = btn.on_press(Message::AddBlockModel(c.index));
            }
            list = list.push(btn);
        }

        column![
            row![text("Add block — Category:").size(13), cat_picker, button(text("Cancel").size(12)).on_press(Message::ToggleAddPicker)]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            scrollable(list).height(Length::Fixed(240.0)),
        ]
        .spacing(6)
        .into()
    }
}

/// A category option for the picker's `pick_list` (its `Display` is the human name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CatItem {
    id: i64,
    name: &'static str,
}

impl std::fmt::Display for CatItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

/// A split-type option for the routing panel's `pick_list` (`Display` is the label; `index` is the
/// `Helix.sym` model index passed to `swap_model`; `symbol` matches the node's `symbolic_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SplitTypeItem {
    index: i64,
    symbol: &'static str,
    label: &'static str,
}

impl std::fmt::Display for SplitTypeItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}

/// An enum-param option for a param dropdown (its `Display` is the label; `index` is the wire value).
#[derive(Debug, Clone, PartialEq, Eq)]
struct EnumOpt {
    index: usize,
    label: String,
}

impl std::fmt::Display for EnumOpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// The first non-controller block of a preset (the default selection on load).
fn first_block_slot(p: &EditorPreset) -> Option<i64> {
    p.blocks.iter().find(|b| !b.is_controller).map(|b| b.slot)
}

/// Lay a block's params out in two columns of compact cells.
fn param_grid(slot: i64, paired: bool, params: &[EditorParam], connected: bool) -> Element<'_, Message> {
    let mut grid = column![].spacing(8);
    let mut pair: Vec<Element<Message>> = Vec::with_capacity(2);
    for prm in params {
        pair.push(param_cell(slot, paired, prm, connected));
        if pair.len() == 2 {
            let mut r = row![].spacing(16);
            for cell in pair.drain(..) {
                r = r.push(container(cell).width(Length::FillPortion(1)));
            }
            grid = grid.push(r);
        }
    }
    if let Some(cell) = pair.pop() {
        grid = grid.push(row![
            container(cell).width(Length::FillPortion(1)),
            container(text("")).width(Length::FillPortion(1)),
        ].spacing(16));
    }
    grid.into()
}

/// One param as a compact cell: name on top, a slider + readout below. A live float param with a
/// numeric range is editable (commit on release); everything else is a read-only readout.
fn param_cell(slot: i64, paired: bool, prm: &EditorParam, connected: bool) -> Element<'_, Message> {
    let index = prm.index;
    let name = text(prm.name.clone()).size(13);

    // Enum param (valueType 0) with known labels → a dropdown of those labels; the value is the
    // selected index. Read-only (a plain readout of the current label) when not connected.
    if prm.meta.value_type == Some(0) && !prm.meta.enum_labels.is_empty() {
        let labels = &prm.meta.enum_labels;
        let cur = match prm.value {
            ParamValue::Int(i) => i,
            ParamValue::Float(f) => f.round() as i64,
            ParamValue::Bool(b) => b as i64,
        };
        let cur_label = labels.get(cur as usize).cloned().unwrap_or_else(|| format!("#{cur}"));
        if connected {
            let opts: Vec<EnumOpt> = labels
                .iter()
                .enumerate()
                .map(|(i, l)| EnumOpt { index: i, label: l.clone() })
                .collect();
            let selected = opts.get(cur as usize).cloned();
            let pl = pick_list(opts, selected, move |o: EnumOpt| {
                Message::ParamEnumSelected(slot, paired, index, o.index as i64)
            })
            .text_size(13);
            return column![name, pl].spacing(2).into();
        }
        return column![name, text(cur_label).size(13)].spacing(2).into();
    }

    let editable = match (connected, prm.value, prm.meta.min, prm.meta.max) {
        (true, ParamValue::Float(_), Some(min), Some(max)) if max > min => Some((min, max)),
        _ => None,
    };
    if let Some((min, max)) = editable {
        let value = fmt_f32(prm.value);
        let step = ((max - min) as f32 / 200.0).max(f32::MIN_POSITIVE);
        let s = slider(min as f32..=max as f32, value, move |v| {
            Message::ParamChanged(slot, paired, index, v)
        })
            .step(step)
            .on_release(Message::ParamCommitted(slot, paired, index));
        column![name, row![s, text(fmt(prm.value)).size(13)].spacing(8)].spacing(2).into()
    } else {
        column![name, text(fmt(prm.value)).size(13)].spacing(2).into()
    }
}

/// Run blocking device work off the UI thread (iced's tokio executor), flattening the join error.
fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> impl Future<Output = Result<T, String>> + Send {
    async move {
        tokio::task::spawn_blocking(f)
            .await
            .unwrap_or_else(|e| Err(format!("worker panicked: {e}")))
    }
}

/// Decode a `dump-raw` preset stream file into the editor model via the reference catalog.
fn load(path: &str) -> Result<EditorPreset, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Catalog::load()
        .map_err(|e| e.to_string())?
        .load_preset(&bytes)
        .map_err(|e| e.to_string())
}

fn fmt(v: ParamValue) -> String {
    match v {
        ParamValue::Float(f) => format!("{f}"),
        ParamValue::Int(i) => format!("{i}"),
        ParamValue::Bool(b) => format!("{b}"),
    }
}

/// A value coerced to the `f32` the wire `set_value` op expects.
fn fmt_f32(v: ParamValue) -> f32 {
    match v {
        ParamValue::Float(f) => f,
        ParamValue::Int(i) => i as f32,
        ParamValue::Bool(b) => b as i32 as f32,
    }
}

