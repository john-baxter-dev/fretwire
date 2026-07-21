//! Serde wire types handed to the Svelte frontend. Kept in the Tauri layer (not derived on the
//! `fretwire-core` model) so the web-facing shape is decoupled from the editor's internal types and can
//! evolve independently. Each is a thin `From<&…>` projection of the corresponding `fretwire-core` value.

use fretwire_core::fretwire_data::stream::{ParamValue, StatusPush};
use fretwire_core::{EditorBlock, EditorParam, EditorPreset, ModelChoice};
use serde::Serialize;

/// A device-originated state change (footswitch bypass, panel snapshot/preset switch), forwarded to
/// the frontend so the GUI follows the hardware live. `Other` pushes are dropped.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind")]
pub enum PushDto {
    Bypass { slot: i64, enabled: bool },
    Snapshot { index: i64 },
    Preset { index: i64 },
}

pub fn push_dtos(pushes: &[StatusPush]) -> Vec<PushDto> {
    pushes
        .iter()
        .filter_map(|p| match p {
            StatusPush::Bypass { slot, enabled } => {
                Some(PushDto::Bypass { slot: *slot, enabled: *enabled })
            }
            StatusPush::Snapshot(i) => Some(PushDto::Snapshot { index: *i }),
            StatusPush::Preset(i) => Some(PushDto::Preset { index: *i }),
            StatusPush::Other(_) => None,
        })
        .collect()
}

/// JSON can't represent NaN/Infinity — `serde_json` errors on them, which for an async Tauri command
/// leaves the invoke promise hanging. Coerce any non-finite float to a safe finite value.
fn fin(x: f64) -> f64 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
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
}

#[derive(Serialize)]
pub struct SegStopDto {
    pub value: f64,
    pub label: String,
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
                .map(|s| SegStopDto { value: fin(s.value), label: s.label.clone() })
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub struct BlockDto {
    pub slot: i64,
    /// 0 = main (top) row, 1 = parallel (B / bottom) row.
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
    pub slot: i64,
    pub row: u8,
    pub column: i64,
    pub occupied: bool,
}

#[derive(Serialize)]
pub struct PresetDto {
    pub name: Option<String>,
    pub index: Option<i64>,
    pub bank: Option<i64>,
    pub device_model: Option<String>,
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
            firmware: p.firmware.clone(),
            split: p.split,
            dsp_load: fin(p.dsp_load),
            split_pos: p.split_pos,
            mixer_pos: p.mixer_pos,
            active_snapshot: p.active_snapshot,
            snapshot_names: p.snapshot_names.clone(),
            blocks: p.blocks.iter().map(BlockDto::from).collect(),
            split_node: p.split_node.as_ref().map(BlockDto::from),
            mixer_node: p.mixer_node.as_ref().map(BlockDto::from),
            input_node: p.input_node.as_ref().map(BlockDto::from),
            output_node: p.output_node.as_ref().map(BlockDto::from),
            grid: p
                .grid
                .iter()
                .map(|c| GridCellDto { slot: c.slot, row: c.row, column: c.column, occupied: c.occupied })
                .collect(),
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
