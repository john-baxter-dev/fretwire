//! Serde wire types handed to the Svelte frontend. Kept in the Tauri layer (not derived on the
//! `fretwire-core` model) so the web-facing shape is decoupled from the editor's internal types and can
//! evolve independently. Each is a thin `From<&…>` projection of the corresponding `fretwire-core` value.

use fretwire_core::fretwire_data::stream::{ParamValue, StatusPush};
use fretwire_core::{EditorBlock, EditorParam, EditorPreset, ModelChoice};
use serde::Serialize;

/// A device-originated state change (footswitch bypass, panel snapshot/preset switch, panel knob),
/// forwarded to the frontend so the GUI follows the hardware live. `Other` pushes are dropped.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind")]
pub enum PushDto {
    Bypass {
        slot: i64,
        enabled: bool,
    },
    Snapshot {
        index: i64,
    },
    Preset {
        index: i64,
    },
    Param {
        slot: i64,
        param: i64,
        value: f64,
        /// `true` when `param` is an index into the block's extra values, matching a
        /// [`ParamDto::extra_index`] rather than a [`ParamDto::index`].
        extra: bool,
        /// `true` when the change is on the block's paired cab/IR rather than its own model — the
        /// two param lists both start at 0, so this picks which one `param` indexes.
        paired: bool,
    },
}

pub fn push_dtos(pushes: &[StatusPush]) -> Vec<PushDto> {
    pushes
        .iter()
        .filter_map(|p| match p {
            StatusPush::Bypass { slot, enabled } => Some(PushDto::Bypass {
                slot: *slot,
                enabled: *enabled,
            }),
            StatusPush::Snapshot(i) => Some(PushDto::Snapshot { index: *i }),
            StatusPush::Preset(i) => Some(PushDto::Preset { index: *i }),
            // The frontend renders every parameter as a number, so flatten the three wire types the
            // same way the param DTOs do rather than teaching the UI a tagged value.
            StatusPush::Param {
                slot,
                param,
                value,
                extra,
                paired,
            } => Some(PushDto::Param {
                slot: *slot,
                param: *param,
                extra: *extra,
                paired: *paired,
                value: fin(match value {
                    ParamValue::Float(f) => *f as f64,
                    ParamValue::Int(i) => *i as f64,
                    ParamValue::Bool(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                }),
            }),
            StatusPush::Idle | StatusPush::Other(_) => None,
        })
        .collect()
}

/// JSON can't represent NaN/Infinity — `serde_json` errors on them, which for an async Tauri command
/// leaves the invoke promise hanging. Coerce any non-finite float to a safe finite value.
fn fin(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

fn fin_opt(x: Option<f64>) -> Option<f64> {
    x.map(|v| if v.is_finite() { v } else { 0.0 })
}

fn param_num(v: &ParamValue) -> f64 {
    let n = match v {
        ParamValue::Float(f) => *f as f64,
        ParamValue::Int(i) => *i as f64,
        ParamValue::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
    };
    fin(n)
}

fn param_kind(v: &ParamValue) -> &'static str {
    match v {
        ParamValue::Float(_) => "float",
        ParamValue::Int(_) => "int",
        ParamValue::Bool(_) => "bool",
    }
}

#[derive(Serialize, Default)]
pub struct ParamDto {
    /// Wire selector (edit target key 28) — pass back to `set_param`.
    pub index: usize,
    pub name: String,
    /// Numeric value (bools as 0/1); `kind` says how to interpret it.
    pub value: f64,
    pub kind: &'static str,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// `.models` valueType: 0 = enum dropdown, 1 = float knob, 2 = bool switch.
    pub value_type: Option<i64>,
    pub display_type: Option<String>,
    /// For enum params: ordered option labels. The written value is the label's position **plus
    /// `enum_base`** — see below.
    pub enum_labels: Vec<String>,
    /// The wire value of `enum_labels[0]`: a discrete control's labels span the param's `min..=max`,
    /// which is not always 0 (`Note Sync` is 19 note values over 1..=19). Rendering the list from 0
    /// showed the note after the one on the pedal and wrote the one before the one picked
    /// [issue #8]. See [`fretwire_core::editor::ParamMeta::enum_base`].
    pub enum_base: i64,
    /// For segmented floats (cab mic Angle: 0°/45°): the discrete stops — render as buttons; the
    /// stop's `value` is written via the ordinary float path. Empty for continuous params.
    pub stops: Vec<SegStopDto>,
    /// `false` when op 30 cannot address this param at all (see [`EditorParam::settable`]) — show
    /// the value, but no control.
    pub settable: bool,
    /// Set for a value addressed in the block's **extra** space rather than the model's param list
    /// (`Trails`, a legacy cab's mic index). A device push carrying `extra: true` matches on this
    /// number; one carrying `extra: false` matches on `index`. Both spaces start at 0, so matching
    /// on the wrong one silently drives a different control.
    pub extra_index: Option<i64>,
    /// How to render this value with its unit. Sent as rules rather than a finished string because
    /// the panel re-formats continuously while a slider is dragged, before any value reaches Rust.
    pub format: Option<NumFormatDto>,
    /// One fine increment in stored units — what a scroll-wheel notch moves. `null` where the
    /// reference data gives a range-switched step; see [`fretwire_core::editor::ParamMeta::step`].
    pub step: Option<f64>,
    /// The model's default value, in stored units — what a double-click on the control resets to.
    pub default: Option<f64>,
}

#[derive(Serialize, Default)]
pub struct SegStopDto {
    pub value: f64,
    pub label: String,
}

/// Display recipe for a continuous param — see [`fretwire_core::editor::NumFormat`]. The panel
/// applies `scale` and `offset`, picks the first rule bracketing the result, multiplies by its
/// `mult`, and fills the printf-ish `template`.
#[derive(Serialize, Default)]
pub struct NumFormatDto {
    pub scale: f64,
    /// Added after `scale`; non-zero only for the bipolar controls (pan: ×200 − 100).
    pub offset: f64,
    pub rules: Vec<FormatRuleDto>,
}

#[derive(Serialize, Default)]
pub struct FormatRuleDto {
    /// `null` for an unbounded end — JSON has no infinity.
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub mult: f64,
    pub template: String,
}

impl From<&EditorParam> for ParamDto {
    fn from(p: &EditorParam) -> Self {
        ParamDto {
            index: p.index,
            // The display name, not the `symbolicID` we address it by — see
            // `EditorParam::display_name`.
            name: p.display_name().to_string(),
            value: param_num(&p.value),
            kind: param_kind(&p.value),
            min: fin_opt(p.meta.min),
            max: fin_opt(p.meta.max),
            value_type: p.meta.value_type,
            display_type: p.meta.display_type.clone(),
            enum_labels: p.meta.enum_labels.clone(),
            enum_base: p.meta.enum_base(),
            stops: p
                .meta
                .stops
                .iter()
                .map(|s| SegStopDto {
                    value: fin(s.value),
                    label: s.label.clone(),
                })
                .collect(),
            settable: p.settable,
            extra_index: p.extra_index,
            step: fin_opt(p.meta.step),
            default: fin_opt(p.meta.default),
            format: p.meta.format.as_ref().map(|f| NumFormatDto {
                scale: f.scale,
                offset: f.offset,
                rules: f
                    .rules
                    .iter()
                    .map(|r| FormatRuleDto {
                        lo: fin_opt(Some(r.lo)),
                        hi: fin_opt(Some(r.hi)),
                        mult: r.mult,
                        template: r.template.clone(),
                    })
                    .collect(),
            }),
        }
    }
}

#[derive(Serialize, Default)]
pub struct BlockDto {
    /// Wire slot — global across DSPs (`dsp * 20 + index`), and the edit address.
    pub slot: i64,
    /// Which DSP holds this block (0 for every HX Stomp block).
    pub dsp: usize,
    /// 0 = main (top) row, 1 = parallel (B / bottom) row, **within this block's DSP**.
    pub row: u8,
    pub model_name: String,
    pub user_label: Option<String>,
    /// Custom footswitch LED colour on this block's switch — HX Edit's palette index, not RGB.
    pub custom_color: Option<i64>,
    pub symbolic_id: Option<String>,
    pub category: Option<i64>,
    pub bypassed: Option<bool>,
    pub variant: Option<String>,
    pub is_controller: bool,
    pub footswitch: i64,
    pub dsp_load: Option<f64>,
    pub params: Vec<ParamDto>,
    /// The block's own `Helix.sym` index — pass back to `swap_model` when changing only the
    /// paired cab.
    pub model_index: Option<i64>,
    pub paired_model_name: Option<String>,
    pub paired_index: Option<i64>,
    pub paired_symbolic_id: Option<String>,
    pub paired_category: Option<i64>,
    pub paired_params: Vec<ParamDto>,
}

impl From<&EditorBlock> for BlockDto {
    fn from(b: &EditorBlock) -> Self {
        BlockDto {
            slot: b.slot,
            dsp: b.dsp,
            row: b.row,
            model_name: b.model_name.clone(),
            user_label: b.user_label.clone(),
            custom_color: b.custom_color,
            symbolic_id: b.symbolic_id.clone(),
            category: b.category,
            bypassed: b.bypassed,
            variant: b.variant.map(str::to_string),
            is_controller: b.is_controller,
            footswitch: b.footswitch,
            dsp_load: fin_opt(b.dsp_load),
            params: b.params.iter().map(ParamDto::from).collect(),
            model_index: b.model_index,
            paired_model_name: b.paired_model_name.clone(),
            paired_index: b.paired_index,
            paired_symbolic_id: b.paired_symbolic_id.clone(),
            paired_category: b.paired_category,
            paired_params: b.paired_params.iter().map(ParamDto::from).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct GridCellDto {
    /// Which DSP this cell belongs to (0 for every HX Stomp cell).
    pub dsp: usize,
    /// Wire slot — global across DSPs, and the drop-target address.
    pub slot: i64,
    /// Row **within** this DSP: 0 = path A, 1 = path B.
    pub row: u8,
    pub column: i64,
    pub occupied: bool,
}

impl From<&fretwire_core::fretwire_data::stream::GridCell> for GridCellDto {
    fn from(c: &fretwire_core::fretwire_data::stream::GridCell) -> Self {
        GridCellDto {
            dsp: c.dsp,
            slot: c.slot,
            row: c.row,
            column: c.column,
            occupied: c.occupied,
        }
    }
}

/// One DSP's routing structure. A single-DSP device (HX Stomp) sends one of these; the Helix Floor
/// sends two. The flat `split_node`/`grid`/… fields on [`PresetDto`] mirror `dsps[0]` so a UI that
/// only knows about one DSP keeps working.
#[derive(Serialize)]
pub struct DspDto {
    pub dsp: usize,
    pub split: bool,
    pub split_pos: Option<i64>,
    pub mixer_pos: Option<i64>,
    pub split_node: Option<BlockDto>,
    pub mixer_node: Option<BlockDto>,
    pub input_node: Option<BlockDto>,
    pub output_node: Option<BlockDto>,
    pub grid: Vec<GridCellDto>,
    /// DSP load drawn by this DSP's blocks alone (each DSP has its own ~100% budget).
    pub dsp_load: f64,
}

#[derive(Serialize, Default)]
pub struct PresetDto {
    pub name: Option<String>,
    pub index: Option<i64>,
    pub bank: Option<i64>,
    /// Model code stamped into the preset by the device that wrote it (e.g. `"P33"`).
    pub device_model: Option<String>,
    /// Human name of the **connected** device, from the USB PID — stamped by the command layer
    /// (`From` leaves it `None`, since an offline decode has no device).
    pub device_name: Option<String>,
    /// `false` only when the connected device's model code and the preset's disagree; `None` when
    /// either is unknown. Stamped by the command layer.
    pub device_matches: Option<bool>,
    /// The preset's build stamp. Deliberately not called `firmware`: it does not track the
    /// pedal's firmware version, and labelling it that way had a tester reasonably reading it as
    /// fretwire misreporting their device. See `PresetStream::build_stamp`.
    pub build_stamp: Option<String>,
    pub split: bool,
    pub dsp_load: f64,
    /// The load a DSP fills up at, on `dsp_load`'s scale — **~75, not 100** (see
    /// `fretwire_core::editor::DSP_CEILING`). Sent so the UI's meters and "does it fit" greying
    /// read the measured ceiling instead of carrying their own copy of the number.
    pub dsp_ceiling: f64,
    pub split_pos: Option<i64>,
    pub mixer_pos: Option<i64>,
    pub active_snapshot: Option<i64>,
    pub snapshot_names: Vec<String>,
    pub blocks: Vec<BlockDto>,
    pub split_node: Option<BlockDto>,
    pub mixer_node: Option<BlockDto>,
    /// The fixed input (slot 0: gate/threshold/decay) and output (slot 9: pan/level) nodes —
    /// edited with the ordinary param commands on their slots.
    pub input_node: Option<BlockDto>,
    pub output_node: Option<BlockDto>,
    pub grid: Vec<GridCellDto>,
    /// Every populated DSP, in order — one entry on the HX Stomp, two on the Helix Floor. The flat
    /// fields above mirror `dsps[0]`; a multi-DSP UI should read this instead.
    pub dsps: Vec<DspDto>,
    /// Undo/redo stack depths — stamped by the command layer (`From` leaves them 0), so the UI can
    /// enable/disable its history buttons.
    pub undo_depth: i64,
    pub redo_depth: i64,
    /// Edit-history timeline labels (oldest first) and the cursor into it — stamped by the command
    /// layer; drives the history pane (click an entry → `history_jump`).
    pub history: Vec<String>,
    pub history_cursor: i64,
    /// Every parameter under a controller. A block's bypass binding is **not** here — it is
    /// `BlockDto::footswitch`.
    pub assignments: Vec<AssignmentDto>,
    /// How many footswitch positions this device has (5 on an HX Stomp: three on the panel, two on
    /// the external jack). Read from the preset's own layout, so it is right without the UI knowing
    /// anything about device models.
    pub footswitch_count: usize,
    /// `true` when the edit buffer has changes not saved to flash — stamped by the command layer.
    pub dirty: bool,
}

/// One controller assignment — a parameter driven by an expression pedal, a footswitch, MIDI or
/// snapshots (preset key `4`).
///
/// A block's **bypass** on a footswitch is not one of these: it lives in the footswitch layout and
/// reaches the UI as `BlockDto::footswitch`. The two are written by different opcodes and the UI
/// presents them in different places — see `docs/protocol.md`.
#[derive(Debug, Clone, Serialize)]
pub struct AssignmentDto {
    /// Source ordinal, as `assign_param` takes it: 1-2 expression pedals, then one per footswitch
    /// from 3, then MIDI, then snapshots. Where the run ends is the device's — see
    /// `fretwire_protocol::edit::source`.
    pub source: i64,
    /// How to name the source in the UI (`"FS1"`, `"EXP1"`, `"Snapshots"`).
    pub source_name: String,
    pub target_slot: Option<i64>,
    pub param_index: Option<i64>,
    /// The parameter is on the block's paired cab, so its index is in the cab's own list.
    pub paired: bool,
    /// The parameter's display name, resolved against whichever list `paired` selects. `None` when
    /// the target block or parameter can't be resolved.
    pub param_name: Option<String>,
    /// Travel ends in the parameter's own units. Bools arrive as 0/1, matching `ParamDto::value`.
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Name a source ordinal for display, for a device with `footswitch_count` switches.
///
/// The count is not optional decoration: ordinal `8` is MIDI on a Stomp and **FS6** on an XL, and
/// naming it without one showed an XL owner's front-panel assignment as "Driven by MIDI" (issue
/// #13). See `fretwire_protocol::edit::source`, which owns the arithmetic.
pub fn source_name(ordinal: i64, footswitch_count: usize) -> String {
    fretwire_core::fretwire_protocol::edit::source::name(ordinal, footswitch_count)
}

/// A travel end as a number. The wire keeps these in the parameter's own type — `false`/`true` for
/// a switch, a float for a knob — and the UI wants one numeric scale, the same one `ParamDto::value`
/// uses for bools.
fn travel_num(v: Option<&fretwire_core::fretwire_data::rmpv::Value>) -> Option<f64> {
    let v = v?;
    v.as_f64()
        .or_else(|| v.as_i64().map(|i| i as f64))
        .or_else(|| v.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
}

impl From<&EditorPreset> for PresetDto {
    fn from(p: &EditorPreset) -> Self {
        PresetDto {
            name: p.current.as_ref().map(|c| c.name.clone()),
            index: p.current.as_ref().map(|c| c.index),
            bank: p.current.as_ref().map(|c| c.bank),
            device_model: p.device_model.clone(),
            device_name: None,
            device_matches: None,
            build_stamp: p.build_stamp.clone(),
            split: p.split(),
            dsp_load: fin(p.dsp_load),
            dsp_ceiling: fretwire_core::editor::DSP_CEILING,
            split_pos: p.split_pos(),
            mixer_pos: p.mixer_pos(),
            active_snapshot: p.active_snapshot,
            snapshot_names: p.snapshot_names.clone(),
            blocks: p.blocks.iter().map(BlockDto::from).collect(),
            split_node: p.split_node().map(BlockDto::from),
            mixer_node: p.mixer_node().map(BlockDto::from),
            input_node: p.input_node().map(BlockDto::from),
            output_node: p.output_node().map(BlockDto::from),
            // Flat `grid` stays DSP 0 only, so a single-DSP UI never sees two DSPs' cells collide
            // at the same (row, column). Multi-DSP UIs read `dsps`.
            grid: p
                .dsp(0)
                .map(|d| d.grid.iter().map(GridCellDto::from).collect())
                .unwrap_or_default(),
            dsps: {
                let loads = p.dsp_load_by_dsp();
                p.dsps
                    .iter()
                    .map(|d| DspDto {
                        dsp: d.dsp,
                        split: d.split,
                        split_pos: d.split_pos,
                        mixer_pos: d.mixer_pos,
                        split_node: d.split_node.as_ref().map(BlockDto::from),
                        mixer_node: d.mixer_node.as_ref().map(BlockDto::from),
                        input_node: d.input_node.as_ref().map(BlockDto::from),
                        output_node: d.output_node.as_ref().map(BlockDto::from),
                        grid: d.grid.iter().map(GridCellDto::from).collect(),
                        dsp_load: fin(loads
                            .iter()
                            .find(|(i, _)| *i == d.dsp)
                            .map_or(0.0, |(_, l)| *l)),
                    })
                    .collect()
            },
            undo_depth: 0,
            redo_depth: 0,
            history: Vec::new(),
            history_cursor: 0,
            dirty: false,
            assignments: p
                .assignments
                .iter()
                .map(|a| AssignmentDto {
                    source: a.controller,
                    source_name: source_name(a.controller, p.footswitch_count),
                    target_slot: a.target_slot,
                    param_index: a.param_index,
                    paired: a.paired(),
                    // Resolve the name in the list the assignment actually points at: a cab's
                    // parameter 1 is `Position` where the amp's is `Bass`, so picking the wrong
                    // list names a real parameter that isn't the one being driven.
                    param_name: a.target_slot.zip(a.param_index).and_then(|(slot, idx)| {
                        let b = p.blocks.iter().find(|b| b.slot == slot)?;
                        let list = if a.paired() {
                            &b.paired_params
                        } else {
                            &b.params
                        };
                        list.iter()
                            .find(|q| q.index as i64 == idx)
                            .map(|q| q.name.clone())
                    }),
                    min: travel_num(a.min.as_ref()),
                    max: travel_num(a.max.as_ref()),
                })
                .collect(),
            footswitch_count: p.footswitch_count,
        }
    }
}

#[derive(Serialize)]
pub struct ModelChoiceDto {
    /// `Helix.sym` index — pass to `swap_model`.
    pub index: i64,
    pub symbolic_id: String,
    pub name: String,
    pub category: Option<i64>,
    pub variant: Option<String>,
    pub dsp_load: Option<f64>,
    /// Amp+Cab entries: the suggested cab's `Helix.sym` index to pass as `paired_index`.
    pub default_paired_index: Option<i64>,
}

impl From<&ModelChoice> for ModelChoiceDto {
    fn from(m: &ModelChoice) -> Self {
        ModelChoiceDto {
            index: m.index,
            symbolic_id: m.symbolic_id.clone(),
            name: m.name.clone(),
            category: m.category,
            variant: m.variant.map(str::to_string),
            dsp_load: fin_opt(m.dsp_load),
            default_paired_index: m.default_paired_index,
        }
    }
}

#[derive(Serialize)]
pub struct CategoryDto {
    pub id: i64,
    pub name: String,
    /// HX Edit's own colour for this category, `"#rrggbb"`, read from `HX_ModelCatalog.json` at
    /// runtime. `null` when the reference data has not been imported — the UI keeps its own palette
    /// for that case rather than showing every block grey.
    pub color: Option<String>,
}

#[derive(Serialize)]
pub struct PresetListItem {
    pub index: i64,
    pub name: String,
    /// Which setlist the entry belongs to. Always `0` for a live listing (which is per-setlist
    /// already) and for every version-1 export file.
    #[serde(default)]
    pub bank: i64,
    /// The setlist's name where the export file recorded one — version-1 files didn't.
    #[serde(default)]
    pub setlist: Option<String>,
    /// How the pedal's own screen writes this slot (`09A`), or `null` on a device whose banking we
    /// haven't seen — the UI then shows the slot number. See `Device::preset_label`.
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct SplitTypeDto {
    /// `Helix.sym` index to pass to `set_split_type`.
    pub index: i64,
    pub symbolic_id: String,
    pub label: String,
}

/// Whether the Line 6 reference data has been imported yet — drives the GUI's first-run screen.
/// Without it the editor still works, but blocks and parameters have numeric indices for names.
#[derive(Serialize)]
pub struct DataStatusDto {
    pub present: bool,
    /// Which device families are imported, by label (`"HX Edit"`, `"POD Go Edit"`).
    pub families: Vec<String>,
    /// Where the tool looks (`$FRETWIRE_DATA_DIR`, else `~/.local/share/fretwire/data`).
    pub dir: String,
    /// How many reference files are cached there — a total across families.
    pub files: i64,
}

impl From<fretwire_core::import::DataStatus> for DataStatusDto {
    fn from(s: fretwire_core::import::DataStatus) -> Self {
        DataStatusDto {
            present: s.present,
            families: s.families.iter().map(|f| f.to_string()).collect(),
            dir: s.dir.display().to_string(),
            files: s.files as i64,
        }
    }
}

/// The outcome of a first-run import.
#[derive(Serialize)]
pub struct ImportResultDto {
    /// Which vendor's data was imported — `"HX Edit"` or `"POD Go Edit"`.
    pub family: String,
    pub copied: i64,
    pub dest: String,
    /// Essential files the source didn't contain — the import still succeeded, but the catalog
    /// will be incomplete. The GUI surfaces this as a warning rather than an error.
    pub missing: Vec<String>,
}

impl From<fretwire_core::import::ImportSummary> for ImportResultDto {
    fn from(s: fretwire_core::import::ImportSummary) -> Self {
        ImportResultDto {
            family: s.family.to_string(),
            copied: s.copied as i64,
            dest: s.dest.display().to_string(),
            missing: s.missing,
        }
    }
}

/// One HX device seen on the bus by `detect`.
///
/// The name is the device's, not a fixed string: an HX Stomp XL that reports itself correctly in
/// the log but is called "HX Stomp" in the UI reads as fretwire misidentifying the pedal.
#[derive(Serialize)]
pub struct DetectedDeviceDto {
    pub name: String,
    /// How far the support for this device actually goes — `None` for a device whose traffic we
    /// have reconciled, otherwise the sentence to show the user.
    pub caveat: Option<String>,
}

impl From<&'static fretwire_core::fretwire_usb::Device> for DetectedDeviceDto {
    fn from(d: &'static fretwire_core::fretwire_usb::Device) -> Self {
        DetectedDeviceDto {
            name: d.name.to_string(),
            caveat: d.support.caveat().map(str::to_string),
        }
    }
}

/// An IR as a file, for the frontend to save itself (`ir_export_inline`).
#[derive(serde::Serialize, Clone, Debug)]
pub struct IrFileDto {
    /// The name stored in the slot — the natural file stem.
    pub name: String,
    /// The 32-bit float, 48 kHz mono WAV, base64 (standard alphabet, padded).
    pub wav_base64: String,
}

/// An export file handed back instead of written (`export_setlists_inline`).
#[derive(serde::Serialize, Clone, Debug)]
pub struct BackupFileDto {
    /// Presets in the file — what `export_setlists` returns on its own.
    pub count: i64,
    /// The file's text, exactly what the path variant would have written.
    pub json: String,
}

/// One user IR slot, as the IR panel renders it.
///
/// The two listings the device offers answer with different fields — the directory (op 13) carries
/// the stored hash but no checksum or length, a per-slot sweep the reverse — so everything past the
/// index and name is optional, and the UI shows a column only where it has something to put there.
#[derive(serde::Serialize, Clone, Debug)]
pub struct IrSlotDto {
    /// Zero-based slot index. The device numbers these from 1 in its own menus, so the UI adds one.
    pub index: i64,
    /// The stored name, empty for an unused slot.
    pub name: String,
    /// What to print: the name, or a stand-in for the nameless and the empty.
    pub display_name: String,
    /// Whether the slot holds an IR — the device's declared length, not the presence of a name.
    pub used: bool,
    /// Samples the device stores here, `0` for an empty slot.
    pub samples: i64,
    /// The `113` word sum, when this listing carries one.
    pub checksum: Option<u32>,
    /// The MD5 of the stored bytes, when this listing carries one.
    pub md5: Option<String>,
}

impl From<&fretwire_core::fretwire_data::ir::IrSlot> for IrSlotDto {
    fn from(s: &fretwire_core::fretwire_data::ir::IrSlot) -> Self {
        Self {
            index: s.index,
            name: s.name.clone(),
            display_name: s.display_name().to_string(),
            used: s.is_used(),
            samples: s.stored_samples(),
            checksum: s.checksum,
            md5: s.md5.clone(),
        }
    }
}

#[cfg(test)]
mod ir_tests {
    use super::IrSlotDto;
    use fretwire_core::fretwire_data::ir::{IrFlags, IrSlot};

    fn slot(name: &str, mul: i64, exp: i64) -> IrSlot {
        IrSlot {
            index: 4,
            checksum: Some(0xc0a0_76ed),
            name: name.to_string(),
            md5: Some("4b41c57b04c05b1471277ecf74231a7d".into()),
            length_mul: mul,
            length_exp: exp,
            flags: IrFlags::default(),
        }
    }

    /// The panel reads these keys by name. A rename here compiles, passes the JS mock (whose keys
    /// are hand-written), and then silently renders `undefined` in the real app — so the wire
    /// names are pinned rather than left to `serde`'s defaults.
    #[test]
    fn the_json_keys_are_the_ones_the_panel_reads() {
        let json = serde_json::to_value(IrSlotDto::from(&slot("Greenback", 1, 3))).unwrap();
        let obj = json.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "checksum",
                "display_name",
                "index",
                "md5",
                "name",
                "samples",
                "used"
            ]
        );
        assert_eq!(obj["samples"], 2048);
        assert_eq!(obj["used"], true);
        assert_eq!(obj["display_name"], "Greenback");
    }

    #[test]
    fn an_empty_slot_carries_the_dash_and_no_length() {
        let dto = IrSlotDto::from(&IrSlot {
            checksum: None,
            md5: None,
            name: String::new(),
            length_mul: 0,
            length_exp: 1,
            ..slot("", 0, 1)
        });
        assert!(!dto.used);
        assert_eq!(dto.samples, 0);
        assert_eq!(dto.display_name, "—");
    }

    #[test]
    fn a_nameless_but_occupied_slot_is_still_occupied() {
        // The case the panel tags "silent": no name, but the device declares a stored length.
        // Reading occupancy off the name would offer this slot as free space it is not.
        let dto = IrSlotDto::from(&slot("", 1, 3));
        assert!(dto.used);
        assert_eq!(dto.display_name, "(unnamed)");
        assert_eq!(dto.samples, 2048);
    }
}

/// One device setting, as the globals panel shows it.
///
/// Covers both the identified ids (from `fretwire_protocol::settings`) and the ones that merely
/// answer — those come back with `kind: "raw"` and `writable: false`, because a value nobody has
/// explained is worth showing and is not worth writing. See `settings::is_writable`.
#[derive(Serialize, Clone, Debug)]
pub struct SettingDto {
    pub id: i64,
    /// The pedal's name for it, or `"Setting <id>"` where we have none.
    pub name: String,
    pub group: String,
    /// `"flag"`, `"choice"`, `"number"` or `"raw"` — how the panel should render it.
    pub kind: String,
    /// The current value: a JSON bool, integer or float, mirroring what the device holds.
    pub value: serde_json::Value,
    /// For `"flag"`: the menu labels for `true` and `false`, in that order.
    pub labels: Option<[String; 2]>,
    /// For `"choice"`: the observed values and their names. May be empty for a setting whose values
    /// have been seen but never explained.
    pub options: Vec<(i64, String)>,
    /// For `"number"`: the unit, possibly empty.
    pub unit: String,
    /// For `"number"`: the sentinel meaning "off", where the device uses one.
    pub off: Option<f64>,
    /// The pedal's factory value, where we have watched it reset one — the EQ and nothing else.
    /// Drives the panel's reset controls; `null` means no reset is offered, not "reset to zero".
    pub default: Option<f64>,
    pub writable: bool,
}

impl SettingDto {
    /// Re-label anything the **connected** device spells differently from the static table.
    ///
    /// `settings::SETTINGS` is one flat table with no notion of which pedal is plugged in, which is
    /// right for every setting whose menu text is fixed. Id 27 is the exception: its menu spells out
    /// the preset range, so a Stomp draws `000-125`/`01A-42C` where an XL draws `000-127`/`01A-32D`.
    /// Shipping one pair means telling half the users something their own screen contradicts, and
    /// that is the failure `Device::presets_per_bank` already warns about.
    ///
    /// A device whose bank size has never been measured keeps the table's text — there is nothing to
    /// improve it with, and the generic label beats an invented one.
    pub fn for_device(mut self, device: &fretwire_core::fretwire_protocol::Device) -> Self {
        // `labels` is `[true, false]`, and id 27 is `true` for the flat form — see
        // `ui/src/lib/numbering.svelte.js`, which reads the same setting.
        if self.id == 27
            && let Some((flat, banked)) = device.preset_numbering_labels()
        {
            self.labels = Some([flat, banked]);
        }
        self
    }

    /// Project one id and the value the device just gave for it.
    pub fn new(id: i64, value: &fretwire_core::fretwire_data::rmpv::Value) -> Self {
        use fretwire_core::fretwire_protocol::settings::{Kind, by_id};
        let json = rmpv_to_json(value);
        match by_id(id) {
            None => Self {
                id,
                name: format!("Setting {id}"),
                group: "Unidentified".into(),
                kind: "raw".into(),
                value: json,
                labels: None,
                options: Vec::new(),
                unit: String::new(),
                off: None,
                default: None,
                writable: false,
            },
            Some(s) => {
                let (kind, labels, options, unit, off) = match s.kind {
                    Kind::Flag { on, off } => (
                        "flag",
                        Some([on.to_string(), off.to_string()]),
                        Vec::new(),
                        String::new(),
                        None,
                    ),
                    Kind::Choice(vs) => (
                        "choice",
                        None,
                        vs.iter().map(|(v, n)| (*v, n.to_string())).collect(),
                        String::new(),
                        None,
                    ),
                    Kind::Number { unit, off } => {
                        ("number", None, Vec::new(), unit.to_string(), off)
                    }
                };
                Self {
                    id,
                    name: s.name.to_string(),
                    group: s.group.to_string(),
                    kind: kind.into(),
                    value: json,
                    labels,
                    options,
                    unit,
                    off,
                    default: fretwire_core::fretwire_protocol::settings::default_of(id),
                    writable: true,
                }
            }
        }
    }
}

/// The device's MessagePack value as JSON, preserving which of the three types it is — the panel
/// renders a bool as a switch and an int as a picker, and `set_setting_num` refuses a write whose
/// type doesn't match what the device already holds.
fn rmpv_to_json(v: &fretwire_core::fretwire_data::rmpv::Value) -> serde_json::Value {
    use fretwire_core::fretwire_data::rmpv::Value;
    match v {
        Value::Boolean(b) => serde_json::Value::Bool(*b),
        Value::Integer(i) => i
            .as_i64()
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
        Value::F32(f) => serde_json::Number::from_f64(f64::from(*f))
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::F64(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.to_string()),
        other => serde_json::Value::String(other.to_string()),
    }
}

#[cfg(test)]
mod setting_tests {
    use super::SettingDto;
    use fretwire_core::fretwire_data::rmpv::Value;

    /// Same contract as the IR DTO's: the panel reads these by name, so a rename here would compile
    /// and then render `undefined`. Mirrored in `ui/tests/ir-mock.mjs`.
    #[test]
    fn the_json_keys_are_the_ones_the_panel_reads() {
        let dto = SettingDto::new(27, &Value::Boolean(true));
        let json = serde_json::to_value(&dto).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "default", "group", "id", "kind", "labels", "name", "off", "options", "unit",
                "value", "writable"
            ]
        );
    }

    #[test]
    fn an_identified_flag_carries_its_menu_labels() {
        let dto = SettingDto::new(27, &Value::Boolean(true));
        assert_eq!(dto.kind, "flag");
        assert_eq!(dto.name, "Preset Number");
        assert_eq!(
            dto.labels,
            Some(["000-127".to_string(), "01A-32D".to_string()])
        );
        assert_eq!(dto.value, serde_json::Value::Bool(true));
        assert!(dto.writable);
    }

    /// One id, one table entry, two pedals that draw it differently. The table can only hold one
    /// pair, so the connected device gets the last word — otherwise a Stomp owner reads `000-127`
    /// off a panel whose own screen says `000-125`.
    #[test]
    fn preset_numbering_is_labelled_for_the_pedal_thats_plugged_in() {
        use fretwire_core::fretwire_protocol::{
            Device, PID_HELIX_FLOOR, PID_HX_STOMP, PID_HX_STOMP_XL,
        };
        let of = |pid| {
            SettingDto::new(27, &Value::Boolean(true))
                .for_device(Device::by_pid(pid).unwrap())
                .labels
                .unwrap()
        };
        assert_eq!(of(PID_HX_STOMP), ["000-125", "01A-42C"]);
        assert_eq!(of(PID_HX_STOMP_XL), ["000-127", "01A-32D"]);
        // The Floor's bank size has never been read off a screen, so there is nothing to substitute
        // and the table's own text stands rather than a guess.
        let table = SettingDto::new(27, &Value::Boolean(true)).labels.unwrap();
        assert_eq!(of(PID_HELIX_FLOOR), table);
    }

    /// Only id 27 is device-dependent so far. If that changes, it changes deliberately.
    #[test]
    fn no_other_setting_is_relabelled_by_the_device() {
        use fretwire_core::fretwire_protocol::{Device, PID_HX_STOMP, settings};
        let stomp = Device::by_pid(PID_HX_STOMP).unwrap();
        for s in settings::SETTINGS.iter().filter(|s| s.id != 27) {
            let plain = SettingDto::new(s.id, &Value::Boolean(true));
            let sent = SettingDto::new(s.id, &Value::Boolean(true)).for_device(stomp);
            assert_eq!(plain.labels, sent.labels, "id {}", s.id);
        }
    }

    /// The point of the raw tier: an id that answers is shown, and is not writable.
    /// The reset controls key off this, so a setting with no observed default must offer none
    /// rather than quietly resetting to zero.
    #[test]
    fn only_the_eq_carries_a_default() {
        assert_eq!(SettingDto::new(192, &Value::F32(0.0)).default, Some(0.0));
        assert_eq!(SettingDto::new(199, &Value::F32(19.9)).default, Some(19.9));
        assert_eq!(SettingDto::new(27, &Value::Boolean(true)).default, None);
        assert_eq!(SettingDto::new(128, &Value::from(3)).default, None);
    }

    #[test]
    fn an_unidentified_id_is_shown_but_not_writable() {
        let dto = SettingDto::new(128, &Value::from(3));
        assert_eq!(dto.kind, "raw");
        assert_eq!(dto.name, "Setting 128");
        assert_eq!(dto.group, "Unidentified");
        assert!(!dto.writable);
    }

    /// Type is preserved rather than flattened to a number — a bool must not arrive as `1`, or the
    /// panel renders a picker where the pedal has a switch and the write is refused with -3.
    #[test]
    fn the_three_value_types_survive_the_projection() {
        assert!(
            SettingDto::new(11, &Value::Boolean(false))
                .value
                .is_boolean()
        );
        assert!(SettingDto::new(14, &Value::from(2)).value.is_i64());
        assert!(SettingDto::new(16, &Value::F32(120.0)).value.is_f64());
    }
}
