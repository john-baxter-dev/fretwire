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
    Bypass { slot: i64, enabled: bool },
    Snapshot { index: i64 },
    Preset { index: i64 },
    Param { slot: i64, param: i64, value: f64 },
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
            StatusPush::Param { slot, param, value } => Some(PushDto::Param {
                slot: *slot,
                param: *param,
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

#[derive(Serialize)]
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
    /// For enum params: ordered option labels; the written value is the index into this list.
    pub enum_labels: Vec<String>,
    /// For segmented floats (cab mic Angle: 0°/45°): the discrete stops — render as buttons; the
    /// stop's `value` is written via the ordinary float path. Empty for continuous params.
    pub stops: Vec<SegStopDto>,
    /// `false` when op 30 cannot address this param at all (see [`EditorParam::settable`]) — show
    /// the value, but no control.
    pub settable: bool,
    /// How to render this value with its unit. Sent as rules rather than a finished string because
    /// the panel re-formats continuously while a slider is dragged, before any value reaches Rust.
    pub format: Option<NumFormatDto>,
}

#[derive(Serialize)]
pub struct SegStopDto {
    pub value: f64,
    pub label: String,
}

/// Display recipe for a continuous param — see [`fretwire_core::editor::NumFormat`]. The panel
/// applies `scale`, picks the first rule bracketing the result, multiplies by its `mult`, and fills
/// the printf-ish `template`.
#[derive(Serialize)]
pub struct NumFormatDto {
    pub scale: f64,
    pub rules: Vec<FormatRuleDto>,
}

#[derive(Serialize)]
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
            name: p.name.clone(),
            value: param_num(&p.value),
            kind: param_kind(&p.value),
            min: fin_opt(p.meta.min),
            max: fin_opt(p.meta.max),
            value_type: p.meta.value_type,
            display_type: p.meta.display_type.clone(),
            enum_labels: p.meta.enum_labels.clone(),
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
            format: p.meta.format.as_ref().map(|f| NumFormatDto {
                scale: f.scale,
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

#[derive(Serialize)]
pub struct BlockDto {
    /// Wire slot — global across DSPs (`dsp * 20 + index`), and the edit address.
    pub slot: i64,
    /// Which DSP holds this block (0 for every HX Stomp block).
    pub dsp: usize,
    /// 0 = main (top) row, 1 = parallel (B / bottom) row, **within this block's DSP**.
    pub row: u8,
    pub model_name: String,
    pub user_label: Option<String>,
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

#[derive(Serialize)]
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
    pub firmware: Option<String>,
    pub split: bool,
    pub dsp_load: f64,
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
    /// `true` when the edit buffer has changes not saved to flash — stamped by the command layer.
    pub dirty: bool,
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
            firmware: p.firmware.clone(),
            split: p.split(),
            dsp_load: fin(p.dsp_load),
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
}

#[derive(Serialize)]
pub struct PresetListItem {
    pub index: i64,
    pub name: String,
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
    /// Where the tool looks (`$FRETWIRE_DATA_DIR`, else `~/.local/share/fretwire/data`).
    pub dir: String,
    /// How many reference files are cached there.
    pub files: i64,
}

impl From<fretwire_core::import::DataStatus> for DataStatusDto {
    fn from(s: fretwire_core::import::DataStatus) -> Self {
        DataStatusDto {
            present: s.present,
            dir: s.dir.display().to_string(),
            files: s.files as i64,
        }
    }
}

/// The outcome of a first-run import.
#[derive(Serialize)]
pub struct ImportResultDto {
    pub copied: i64,
    pub dest: String,
    /// Essential files the source didn't contain — the import still succeeded, but the catalog
    /// will be incomplete. The GUI surfaces this as a warning rather than an error.
    pub missing: Vec<String>,
}

impl From<fretwire_core::import::ImportSummary> for ImportResultDto {
    fn from(s: fretwire_core::import::ImportSummary) -> Self {
        ImportResultDto {
            copied: s.copied as i64,
            dest: s.dest.display().to_string(),
            missing: s.missing,
        }
    }
}
