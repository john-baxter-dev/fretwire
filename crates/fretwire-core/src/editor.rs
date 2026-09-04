//! The editor model: turn a device preset stream into typed, named, editable blocks by composing
//! the lower layers —
//! - `fretwire_data::stream` decodes the MessagePack preset into blocks + values,
//! - `fretwire_data::modeldefs` resolves each block's name to its canonical `symbolicID` + category,
//! - `fretwire_data::symbols` (`Helix.sym`) gives the device's param **order** (Mono/Stereo) so values
//!   get the right names,
//! - `fretwire_protocol::edit` builds the wire commands to change a block.
//!
//! This is the data model a GUI/CLI sits on; it is transport-agnostic (no USB needed to use it).

use fretwire_data::modeldefs::ModelDefs;
use fretwire_data::stream::{ParamValue, PresetStream};
use fretwire_data::symbols::DeviceSymbols;
use std::path::Path;

/// Where a DSP actually stops accepting blocks, on the same scale [`EditorPreset::dsp_load`]
/// reports: **about 75%, not 100%.** Measured on hardware, not derived — so "28% free" on our
/// meter can mean no room at all, and this is the number a fit check must compare against.
///
/// The missing quarter is a **flat reserve, not something the preset spends**. It was long
/// assumed to be the fixed nodes we never sum — the input, the output, and on a parallel preset
/// the split and mixer, which `io.models` does price (`HD2_AppDSPFlow1Input` 10.99,
/// `HD2_AppDSPFlowOutput` 8.00, `HD2_AppDSPFlowSplitY` 1.50, `HD2_AppDSPFlowJoin` 10.99). A
/// census of 363 real presets says otherwise: serial and parallel DSPs stop at the *same* number,
/// so the split and mixer's 12.49 is not billed here, and neither is anything else that varies
/// with the preset. See `docs/protocol.md`. So the raw sum keeps its old meaning — every log,
/// capture note and doc on record quotes it — and the ceiling carries the whole correction.
///
/// Used for *warnings*, never enforcement: the device is the final arbiter (out-guarding it is
/// how the row-B stranding and node-enclosure mistakes happened).
///
/// [solid — three independent lines, 2026-08-02/04.
/// **1.** HX Stomp fw 3.80: one preset, one slot, a ladder of swap targets, by the total each
/// would land on — 73.3 / 73.8 / 74.4 / 74.9 accepted, 75.3 / 75.6 / 76.5 refused `-306`.
/// **2.** The tester's Helix Floor: `somehinged3` sits at 72.7% on DSP1 and refused Elephant Man
/// Mono (+6.02) and Euclidean Delay (+10.5), putting that device's ceiling under 78.7 too.
/// **3.** His `.hxb` backup — 363 presets, 458 DSPs carrying blocks, including Line 6's own two
/// factory setlists. **Nothing exceeds 74.84**, and the wall is flat: 74.84 parallel vs 74.80
/// serial, 74.84 on DSP1 vs 74.77 on DSP2, with 46 presets sitting in the 70–74.84 band. Line 6
/// builds right up to it, which is the best evidence there is that this is the real limit.]
pub const DSP_CEILING: f64 = 75.0;

/// A raw block-load sum as the **percentage of capacity** to show a user: [`DSP_CEILING`] reads
/// 100%. Raw loads are a percentage of a budget the hardware never hands over, so 72.7 reads as
/// plenty of room when it is nearly none — the GUI shows only this scaled figure. The CLI prints
/// both, because its output is what bug reports quote and every load in `docs/` is raw.
pub fn dsp_percent(load: f64) -> f64 {
    load / DSP_CEILING * 100.0
}

/// The shipped reference data needed to interpret a preset: the model table + device param orders.
pub struct Catalog {
    pub models: ModelDefs,
    pub symbols: DeviceSymbols,
    /// `symbolicID` → (mono DSP load, stereo DSP load), as a percentage of the device's DSP budget.
    /// Sourced from the `.models` files' `load`/`load_stereo` fields. Drives the "% DSP" meter.
    loads: std::collections::HashMap<String, (Option<f64>, Option<f64>)>,
    /// Model `symbolicID` → (param `symbolicID` → its UI metadata: range + display type). Sourced
    /// from the `.models` files' per-param `min`/`max`/`displayType`. Lets the editor render the
    /// right control (knob/slider/switch) with the right bounds. A param's `symbolicID` is exactly
    /// the name `Helix.sym` lists in the wire order, so the lookup is by `EditorParam::name`.
    param_meta: std::collections::HashMap<String, std::collections::HashMap<String, ParamMeta>>,
    /// Amp `symbolicID` → its suggested cab `symbolicID` (`amp.models` `ircablink`, the mic+IR cab
    /// engine current firmware uses; `cablink` as fallback). Drives the synthetic "Amp+Cab"
    /// picker category — picking there adds the amp with this cab paired, like HX Edit.
    cab_links: std::collections::HashMap<String, String>,
    /// Category **name** → `0xRRGGBB`, from `HX_ModelCatalog.json`. Keyed by name on purpose: the
    /// catalog's own `id` field is a *different* id space from the one the `.models` files use and
    /// that [`category_name`] speaks — the catalog calls Distortion 1 and Amp 11, where the model
    /// table calls them 3 and 1. Joining on id would silently mispaint every block.
    category_colors: std::collections::HashMap<String, u32>,
    /// Model `symbolicID` → its tempo-sync groups as `[tempo, note, governed]` param symbols,
    /// from `HX_ModelCatalog.json`'s nested param lists. See [`SyncLink`].
    sync_groups: std::collections::HashMap<String, Vec<[String; 3]>>,
}

/// Synthetic picker category: every amp paired with its suggested cab (HX Edit's "Amp+Cab" list).
/// Not a real model-table category id — [`Catalog::models_in_category`] special-cases it.
pub const CATEGORY_AMP_CAB: i64 = 100;

/// Which member of a tempo-sync group a parameter is. See [`SyncLink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncRole {
    /// The `TempoSync{n}` switch: on, and the governed knob becomes a note-value selector.
    Tempo,
    /// The `SyncSelect{n}` note value (`1/4`, `1/8 Dotted`, …) — HX Edit's "Note Sync".
    Note,
    /// The time/rate/speed parameter the pair governs.
    Governed,
}

/// A tempo-sync group: the switch, the note value, and the parameter they take over — three
/// params of one block that HX Edit and the pedal both show as **one control**. Every member
/// carries the same three indices and its own role, so a front end can render the group from any
/// of them: hide the switch and the note, and put both on the governed knob.
///
/// **Which knob a pair governs is stated, not guessed.** `HX_ModelCatalog.json` lists each
/// model's params in HX Edit's display order, and a sync group is a **nested list** —
/// `[{"TempoSync1": null}, {"SyncSelect1": "Note Sync"}, {"Speed": null}]` — the pair followed by
/// the one param it governs. Every nested list in the file has exactly that shape (159 groups
/// over 144 models, 15 of them with two; not one exception), which is what the roadmap had asked
/// for before folding the three rows into one. [solid — the shipped catalog, read 2026-09-03]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncLink {
    pub role: SyncRole,
    /// Param index of the `TempoSync{n}` switch.
    pub tempo: usize,
    /// Param index of the `SyncSelect{n}` note value.
    pub note: usize,
    /// Param index of the parameter the pair governs.
    pub governed: usize,
}

/// UI metadata for one parameter, distilled from a `.models` `Param`: its numeric range and the
/// `displayType` hint (`"generic_knob"`, `"volume"`, `"off_on"`, `"sync_note"`, …) that tells the
/// editor which control to render. All optional — switches and enums carry non-numeric bounds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamMeta {
    /// The tempo-sync group this param belongs to, if any. Set per block from the catalog's
    /// grouping (see [`SyncLink`]) rather than per model, since it names param indices.
    pub sync: Option<SyncLink>,
    /// The param's **display** name from `.models` (`"Note Sync"`), where it differs from the
    /// `symbolicID` we address it by (`"SyncSelect1"`). Most params name themselves the same way
    /// and this stays `None`; the ones that don't were showing raw symbols in the editor.
    /// [issue #5]
    pub label: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub display_type: Option<String>,
    /// The param's `valueType` from `.models`: `0` = integer enum (rendered as a dropdown), `1` =
    /// float knob (slider), `2` = bool switch. `None` if the model didn't describe it.
    pub value_type: Option<i64>,
    /// For an enum param (`value_type == 0`), the ordered option labels. Sourced from
    /// `HelixControls.json[display_type].format` when that control is `isDiscrete` (e.g. the cab
    /// `Mic` selector → "57 Dynamic", "421 Dynamic", …). Empty when the param isn't a known
    /// discrete enum.
    ///
    /// The list spans the param's **`min..=max`**, so the value written is the label's position
    /// plus [`Self::enum_base`] — *not* its bare index. Most enums start at 0 and the two agree;
    /// the ones that don't are why [`Self::enum_label`] exists. See it for the evidence.
    pub enum_labels: Vec<String>,
    /// For a **segmented float** (a float param HX Edit renders as a few discrete positions, e.g.
    /// the cab mic `Angle`: 0°/45°): the allowed stops. The value written is the stop's `value`
    /// via the ordinary float path — the wire type stays float. Empty for continuous params.
    pub stops: Vec<SegStop>,
    /// How to display a continuous value with its unit ("1.373 s" for a stored `1.3728`). `None`
    /// for params whose control isn't described, and for enums/switches, which read as labels.
    /// See [`NumFormat`].
    pub format: Option<NumFormat>,
    /// One fine increment of this param, **in stored units** — `HelixControls.json`'s `step.fine`
    /// (which is in display units) divided by the resolved scale. What one scroll-wheel notch
    /// should move, so a notch is the increment HX Edit and the pedal's own encoder use.
    ///
    /// `None` where the file gives a *range-switched* step instead of one number (delay times,
    /// frequencies — 52 controls do): a single value can't express a curve, and guessing one would
    /// be worse than the caller's own fallback.
    pub step: Option<f64>,
    /// The model's own default for this param, in stored units (`.models` `default`) — pan's `0.5`,
    /// centre. What a double-click on the control resets to. `None` for the routing nodes, whose
    /// params aren't in the `.models` files at all.
    pub default: Option<f64>,
}

impl ParamMeta {
    /// The wire value of the **first** entry in [`Self::enum_labels`] — the param's `min`, or 0
    /// when the model table doesn't bound it.
    ///
    /// A discrete control's label list covers `min..=max`, not `0..=max`: `sync_note` is 19 note
    /// values over `min: 1, max: 19`, `ir_select` is 128 over `1..=128`, the Variax tunings are 25
    /// over `-12..=12`. Every enum in the shipped data whose `min` isn't 0 has exactly
    /// `max - min + 1` labels, so the offset is the rule and not a `sync_note` special case.
    ///
    /// [issue #8 — reading the list from 0 showed the note *after* the one on the pedal's screen,
    /// and writing an index moved the pedal one step short of the note that was picked.]
    pub fn enum_base(&self) -> i64 {
        self.min.filter(|m| m.is_finite()).unwrap_or(0.0) as i64
    }

    /// The label a wire value reads as, or `None` when this param is not a labelled enum or the
    /// value falls outside the list.
    pub fn enum_label(&self, value: i64) -> Option<&str> {
        let i = usize::try_from(value - self.enum_base()).ok()?;
        self.enum_labels.get(i).map(String::as_str)
    }
}

/// One position of a segmented float control (see [`ParamMeta::stops`]).
#[derive(Debug, Clone, PartialEq)]
pub struct SegStop {
    pub value: f64,
    pub label: String,
}

/// One parameter of a block: its index (in the model's device order — this is the wire selector),
/// name (device order when known), and current value.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorParam {
    /// Index in the block's param vector = the wire param selector (edit target key 28).
    pub index: usize,
    pub name: String,
    pub value: ParamValue,
    /// UI metadata (range + display type) for this param, when the model table describes it.
    /// `default` (empty meta) for params the `.models` files don't cover (e.g. the trailing
    /// `Trails` switch). See [`ParamMeta`].
    pub meta: ParamMeta,
    /// Whether an op-30 write can reach this param at all.
    pub settable: bool,
    /// Set for a value the block carries **past the end of its symbol's param list** — `Trails` on
    /// a delay/reverb, the mic index on a legacy cab. These have no index in the model's `Helix.sym`
    /// order, so the ordinary addressing (target key 29 `true`, key 28 = param index) is refused
    /// with `-3`. They are reached instead with key 29 `false`, where key 28 indexes the block's
    /// extra values — and this is that index.
    /// [solid — `captures/dynamic_ambience_trails_on_off.pcapng`, confirmed live on an HX Stomp
    /// 2026-08-02: `{98:2, 29:false, 26:0, 28:0, 119:true}` turns a Bucket Brigade's trails on]
    pub extra_index: Option<i64>,
}

impl EditorParam {
    /// What to show the user. [`Self::name`] is the `symbolicID` — the key everything addresses this
    /// param by — which is usually also the display name, but not always: `SyncSelect1` is HX Edit's
    /// "Note Sync" and `TempoSync1` its "Tempo Sync". Showing the symbol was reported as fretwire
    /// inventing parameters the pedal doesn't have. [issue #5]
    pub fn display_name(&self) -> &str {
        self.meta.label.as_deref().unwrap_or(&self.name)
    }
}

/// A block as the editor sees it: identity, resolved model, current state, named params.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorBlock {
    /// Which DSP holds this block. `0` for every HX Stomp block; a two-DSP device (Helix Floor)
    /// also has `1`. Purely informational for edits — `slot` already encodes it.
    pub dsp: usize,
    /// Preset **wire slot** — global across DSPs (`dsp * 20 + index`), and the edit address
    /// (target key 98). See [`fretwire_data::stream::DSP_SLOT_STRIDE`].
    pub slot: i64,
    /// Display name from the preset (`Harmonic Tremolo`).
    pub model_name: String,
    /// User-assigned block label, if any.
    pub user_label: Option<String>,
    /// Custom footswitch LED colour (a palette index), if one is set on the block's switch.
    pub custom_color: Option<i64>,
    /// Whether the block's footswitch is momentary (hold) rather than latching.
    pub momentary: bool,
    /// Canonical model id, resolved against the model table (`HD2_TremoloHarmonic`).
    pub symbolic_id: Option<String>,
    /// The model's `Helix.sym` index as read from the preset — the wire selector to pass back to
    /// `swap_model` (e.g. to change only the paired cab while keeping this model).
    pub model_index: Option<i64>,
    /// Model category (from the model table).
    pub category: Option<i64>,
    /// Bypass state.
    pub bypassed: Option<bool>,
    /// Device param variant the values matched (`"Mono"`/`"Stereo"`), if resolved.
    pub variant: Option<&'static str>,
    /// `true` if this footswitch-layout node is a controller assignment (`11 → 0 == 2`) rather
    /// than a DSP block — its `model_name` is a footswitch label (e.g. `"OD Sw"`).
    pub is_controller: bool,
    /// Footswitch the block's bypass is bound to (= layout position + 1); `0` = not on a switch.
    pub footswitch: i64,
    /// Signal-path row: 0 = main (top), 1 = parallel (B).
    pub row: u8,
    /// Ordered parameters with names + values, in the model's `Helix.sym` order.
    pub params: Vec<EditorParam>,
    /// Display name of the paired cab/IR fused into this block (amp+cab blocks), if any.
    pub paired_model_name: Option<String>,
    /// `Helix.sym` index of the paired cab/IR (preset `24 → 26`), if any — preserved across a model
    /// swap so changing an amp keeps its cab. `None` = no pairing.
    pub paired_index: Option<i64>,
    /// Canonical model id of the paired cab/IR (base, without the Mono/Stereo variant suffix).
    pub paired_symbolic_id: Option<String>,
    /// The paired cab/IR's model category — drives the "change cab" picker's model list.
    pub paired_category: Option<i64>,
    /// The paired cab/IR's named parameters, if any.
    pub paired_params: Vec<EditorParam>,
    /// DSP load this block draws (% of the device budget), including its paired cab/IR. `None` for
    /// controller nodes and models with no load data.
    pub dsp_load: Option<f64>,
}

impl EditorBlock {
    /// Build the MessagePack edit body that sets this block's **enabled** state (`true` = active,
    /// `false` = bypassed), with counter `txn`.
    /// Wrap with `fretwire_protocol::EditBody::parse(..).to_tlv()` or `fretwire_protocol::Tlv` for the frame.
    pub fn set_enabled_edit(&self, enabled: bool, txn: u16) -> Vec<u8> {
        fretwire_protocol::edit::bypass(self.slot, enabled, txn)
    }

    /// Build a set-value edit for the parameter at `param_index` (its index in this block's param
    /// list, which is the wire selector) to `value` (counter `txn`).
    pub fn set_param_edit(&self, param_index: usize, value: f32, txn: u16) -> Vec<u8> {
        fretwire_protocol::edit::set_value(self.slot, param_index as i64, value, txn)
    }

    /// Build a set-value edit addressing a parameter by name (device order). `None` if not found.
    pub fn set_param_by_name(&self, name: &str, value: f32, txn: u16) -> Option<Vec<u8>> {
        let p = self.params.iter().find(|p| p.name == name)?;
        Some(self.set_param_edit(p.index, value, txn))
    }

    /// Build a set-value edit for the **paired cab/IR's** parameter at `param_index` (its index in
    /// the cab's param list) to `value`. Targets the second model fused into this slot (wire `26:1`),
    /// not the main model. Use for the cab knobs (level, mic distance/position/angle, low/high cut).
    pub fn set_paired_param_edit(&self, param_index: usize, value: f32, txn: u16) -> Vec<u8> {
        fretwire_protocol::edit::set_paired_value(self.slot, param_index as i64, value, txn)
    }
}

/// A decoded, editor-ready preset.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorPreset {
    pub device_model: Option<String>,
    /// The preset's own build stamp — *not* the device's firmware version. See
    /// [`fretwire_data::stream::PresetStream::build_stamp`].
    pub build_stamp: Option<String>,
    pub blocks: Vec<EditorBlock>,
    /// Footswitch/controller assignments (preset key `4`).
    pub assignments: Vec<fretwire_data::stream::Assignment>,
    /// How many footswitch positions this preset's layout has (`3 → 8`'s length) — five on an HX
    /// Stomp, of which three are on the front panel and two reach the external switch jack.
    ///
    /// This is the device's own count, not ours, and it agrees with what the device will accept:
    /// op 33 answers switches 1..=5 and refuses 6 with code `-3` (2026-08-22). Reading it from the
    /// preset means a Floor reports its own number without anything here being told about Floors.
    pub footswitch_count: usize,
    /// Active snapshot index and snapshot names (preset key `10`).
    pub active_snapshot: Option<i64>,
    pub snapshot_names: Vec<String>,
    /// Total DSP load — sum of the blocks' `dsp_load`, and **only** the blocks': the fixed
    /// input/output/split/mixer nodes draw DSP too and are not in here. The device fills up at
    /// [`DSP_CEILING`] on this scale, not 100, so read headroom with
    /// [`EditorPreset::dsp_free_on`] rather than subtracting from 100.
    ///
    /// On a two-DSP device this sums **both** DSPs, which is not how the device budgets them —
    /// each DSP has its own. Use [`EditorPreset::dsp_load_by_dsp`] for the per-DSP figure.
    pub dsp_load: f64,
    /// Identity (bank/index/name) of the preset this edit buffer was read from, when known. Set by
    /// `Session::read_preset` from the op-23 read-info reply; `None` for offline decodes (no device
    /// to ask). The `index` matches `list_presets`/`goto_preset`.
    pub current: Option<fretwire_data::stream::PresetInfo>,
    /// One entry per **populated DSP**, in DSP order — its routing nodes and its grid. A single
    /// entry on the HX Stomp; two on the Helix Floor. The convenience accessors below
    /// ([`Self::split_node`] etc.) read DSP 0, which is what a one-DSP device has.
    pub dsps: Vec<DspView>,
}

/// The routing structure of **one DSP**: its split/mixer/input/output nodes and its grid. A
/// two-DSP device has one of these per DSP, each with its own A/B branch.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DspView {
    /// DSP index (position in [`fretwire_data::stream::DSP_GROUP_KEYS`]).
    pub dsp: usize,
    /// This DSP has a parallel (split) topology — its group key `21` is non-zero.
    pub split: bool,
    /// The parallel **split** node (kind 2) as an editable block, on split presets — its model is the
    /// split *type* ([`SPLIT_TYPES`]) and its params configure that type. `None` on serial presets.
    pub split_node: Option<EditorBlock>,
    /// The parallel **mixer/join** node (kind 3) as an editable block, on split presets — its params
    /// are the A/B level, pan, polarity and master level. `None` on serial presets.
    pub mixer_node: Option<EditorBlock>,
    /// The fixed **input node** (this DSP's slot 0): gate on/off, threshold, decay — edited with
    /// ordinary set-value on its slot. Always present on a well-formed preset.
    pub input_node: Option<EditorBlock>,
    /// The fixed **output node** (this DSP's slot 9): pan, level.
    pub output_node: Option<EditorBlock>,
    /// Signal-flow column of the split node (its holder key `13`): a top-row block at slot `< this` is
    /// common (pre-split), `≥ this` (and `< mixer_pos`) is on **path A**. `None` on serial presets.
    pub split_pos: Option<i64>,
    /// Signal-flow column of the mixer node: a top-row block at slot `≥ this` is common (post-mixer).
    pub mixer_pos: Option<i64>,
    /// This DSP's routing grid: one cell per draggable slot (row/column/occupancy). Drives the
    /// drag-routing UI; see [`fretwire_data::stream::PresetStream::dsp_grid`].
    pub grid: Vec<fretwire_data::stream::GridCell>,
}

impl DspView {
    /// Every routing node this DSP has, in a fixed order.
    fn nodes(&self) -> impl Iterator<Item = &EditorBlock> {
        self.split_node
            .iter()
            .chain(self.mixer_node.iter())
            .chain(self.input_node.iter())
            .chain(self.output_node.iter())
    }

    fn nodes_mut(&mut self) -> impl Iterator<Item = &mut EditorBlock> {
        self.split_node
            .iter_mut()
            .chain(self.mixer_node.iter_mut())
            .chain(self.input_node.iter_mut())
            .chain(self.output_node.iter_mut())
    }
}

impl EditorPreset {
    /// The routing view of one DSP.
    pub fn dsp(&self, dsp: usize) -> Option<&DspView> {
        self.dsps.iter().find(|d| d.dsp == dsp)
    }

    /// DSP 0's split node — the only one a single-DSP device has. See [`DspView::split_node`].
    pub fn split_node(&self) -> Option<&EditorBlock> {
        self.dsp(0).and_then(|d| d.split_node.as_ref())
    }

    /// DSP 0's mixer node. See [`DspView::mixer_node`].
    pub fn mixer_node(&self) -> Option<&EditorBlock> {
        self.dsp(0).and_then(|d| d.mixer_node.as_ref())
    }

    /// DSP 0's input node. See [`DspView::input_node`].
    pub fn input_node(&self) -> Option<&EditorBlock> {
        self.dsp(0).and_then(|d| d.input_node.as_ref())
    }

    /// DSP 0's output node. See [`DspView::output_node`].
    pub fn output_node(&self) -> Option<&EditorBlock> {
        self.dsp(0).and_then(|d| d.output_node.as_ref())
    }

    /// DSP 0's split-node column. See [`DspView::split_pos`].
    pub fn split_pos(&self) -> Option<i64> {
        self.dsp(0).and_then(|d| d.split_pos)
    }

    /// DSP 0's mixer-node column. See [`DspView::mixer_pos`].
    pub fn mixer_pos(&self) -> Option<i64> {
        self.dsp(0).and_then(|d| d.mixer_pos)
    }

    /// Whether **any** DSP has a parallel (split) topology.
    pub fn split(&self) -> bool {
        self.dsps.iter().any(|d| d.split)
    }

    /// DSP load broken down per DSP — `(dsp, load)` in DSP order. Each DSP is budgeted on its own,
    /// so this is the meaningful figure on a two-DSP device; [`Self::dsp_load`] is the whole-preset
    /// sum and means nothing to the hardware.
    pub fn dsp_load_by_dsp(&self) -> Vec<(usize, f64)> {
        self.dsps
            .iter()
            .map(|d| (d.dsp, self.dsp_load_on(d.dsp)))
            .collect()
    }

    /// DSP load drawn by one DSP's blocks. A DSP with no blocks (or no such index) reads `0.0`.
    pub fn dsp_load_on(&self, dsp: usize) -> f64 {
        let load: f64 = self
            .blocks
            .iter()
            .filter(|b| b.dsp == dsp)
            .filter_map(|b| b.dsp_load)
            .sum();
        // An empty f64 sum is -0.0, which formats as "-0.0%" for an unused DSP.
        load + 0.0
    }

    /// Room left on one DSP before the device starts refusing blocks — [`DSP_CEILING`] minus what
    /// that DSP already draws, floored at zero. This, not `100 - load`, is the headroom a user has.
    pub fn dsp_free_on(&self, dsp: usize) -> f64 {
        (DSP_CEILING - self.dsp_load_on(dsp)).max(0.0)
    }

    /// One DSP's load as the percentage of capacity a user should see — the ceiling reads 100%.
    /// See [`dsp_percent`].
    pub fn dsp_percent_on(&self, dsp: usize) -> f64 {
        dsp_percent(self.dsp_load_on(dsp))
    }

    /// Room left per DSP — `(dsp, free)` in DSP order. See [`Self::dsp_free_on`].
    pub fn dsp_free_by_dsp(&self) -> Vec<(usize, f64)> {
        self.dsps
            .iter()
            .map(|d| (d.dsp, self.dsp_free_on(d.dsp)))
            .collect()
    }

    /// Every DSP's grid cells concatenated, in DSP order. Each cell carries its own `dsp`, so a
    /// two-DSP device's four rows are `(cell.dsp, cell.row)`. Identical to DSP 0's grid on a
    /// single-DSP device.
    pub fn grid(&self) -> Vec<fretwire_data::stream::GridCell> {
        self.dsps
            .iter()
            .flat_map(|d| d.grid.iter().cloned())
            .collect()
    }

    /// Find a block by wire slot, **including** the routing nodes of every DSP (which live outside
    /// `blocks`). Lets the param-editing UI treat a node's params the same as any block's.
    pub fn block(&self, slot: i64) -> Option<&EditorBlock> {
        self.blocks
            .iter()
            .chain(self.dsps.iter().flat_map(DspView::nodes))
            .find(|b| b.slot == slot)
    }

    /// Mutable [`Self::block`].
    pub fn block_mut(&mut self, slot: i64) -> Option<&mut EditorBlock> {
        self.blocks
            .iter_mut()
            .chain(self.dsps.iter_mut().flat_map(DspView::nodes_mut))
            .find(|b| b.slot == slot)
    }

    /// Whether `slot` is a split routing node (selecting it offers the split-type picker).
    pub fn is_split_node(&self, slot: i64) -> bool {
        self.dsps
            .iter()
            .any(|d| d.split_node.as_ref().is_some_and(|n| n.slot == slot))
    }

    /// Whether `slot` is a mixer/join routing node.
    pub fn is_mixer_node(&self, slot: i64) -> bool {
        self.dsps
            .iter()
            .any(|d| d.mixer_node.as_ref().is_some_and(|n| n.slot == slot))
    }
}

/// The name the device gives a snapshot nobody has renamed — `SNAPSHOT 1`, `SNAPSHOT 2`, … The
/// index is 0-based, as `EditorPreset::snapshot_names` reports it; the name is 1-based.
///
/// [solid — read off two never-touched preset slots on an HX Stomp, 2026-08-26. The count is the
/// preset's own (`snapshot_names().len()`), three on a Stomp — this only names them.]
pub fn default_snapshot_name(index: usize) -> String {
    format!("SNAPSHOT {}", index + 1)
}

/// The four split-node types the HX Stomp offers, as `(Helix.sym index, canonical symbol, label)`,
/// in the device's menu order. Selecting one is a `swap_model` on the split node's slot (op 40); each
/// type has its own params. Types A/B, Crossover and Dynamic come from `cycle_through_split_types`
/// (op 40 → 256/258/563); **Y (257)** is the default a new split gets — seen stored in
/// `split_preset_stream` — which the cycle capture started on and so never selected.
pub const SPLIT_TYPES: &[(i64, &str, &str)] = &[
    (257, "HD2_AppDSPFlowSplitY", "Y"),
    (256, "HD2_AppDSPFlowSplitAB", "A/B"),
    (258, "HD2_AppDSPFlowSplitXOver", "Crossover"),
    (563, "HD2_AppDSPFlowSplitDyn", "Dynamic"),
];

/// Fold `.models` category ids that HX Edit presents as one picker entry.
///
/// Category **5** holds exactly one model — `HD2_SynthSubtractive`, the 3 Osc Synth — while every
/// other synth sits in 7. HX Edit files it under **Pitch/Synth › Stereo**
/// (`HX_ModelCatalog.json` `categories[7].subcategories[1]`), so a separate "Synth" entry is ours,
/// not the device's: it showed up as a category containing one model, with the 3 Osc Synth missing
/// from the Pitch/Synth list where a user would look for it. [solid — 2026-08-01, from the shipped
/// catalog; reported from the field the same day.]
fn canonical_category(id: i64) -> i64 {
    match id {
        5 => 7,
        other => other,
    }
}

/// Human name for a `.models` `category` id. This is the **device effect-type** enum (distinct from
/// `HX_ModelCatalog.json`'s ids), derived from the shipped `.models` files: each id maps to one
/// effect type. Unknown ids fall back to `None`.
pub fn category_name(id: i64) -> Option<&'static str> {
    let id = canonical_category(id);
    Some(match id {
        1 => "Amp",
        2 => "Cab",
        3 => "Distortion",
        4 => "Dynamics",
        6 => "Filter",
        7 => "Pitch/Synth",
        8 => "Modulation",
        9 => "Delay",
        10 => "Reverb",
        11 => "Wah",
        12 => "Send/Return",
        13 => "Preamp",
        14 => "EQ",
        15 => "Looper",
        16 => "IR",
        17 => "Volume/Pan",
        19 => "Cab (Mic+IR)",
        CATEGORY_AMP_CAB => "Amp+Cab",
        _ => return None,
    })
}

/// A model a block can be swapped to: its `Helix.sym` index (the wire selector for `swap_model`),
/// resolved name, category, variant, and DSP cost.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelChoice {
    /// `Helix.sym` index — pass to `swap_model` / `Session::swap_model`.
    pub index: i64,
    pub symbolic_id: String,
    pub name: String,
    pub category: Option<i64>,
    pub variant: Option<&'static str>,
    /// DSP load (% of budget) for this model+variant, if the model table declares one. For an
    /// Amp+Cab entry this includes the paired cab's cost.
    pub dsp_load: Option<f64>,
    /// For the synthetic Amp+Cab category: the `Helix.sym` index of the suggested cab — pass as
    /// `paired_index` to add/swap so the amp arrives with its matched cab, like HX Edit.
    pub default_paired_index: Option<i64>,
}

/// The reference files the catalog needs, as raw bytes — sourced either from the user's imported
/// data dir ([`Catalog::from_data_dir`]) or, in dev/test builds, embedded ([`Catalog::bundled`]).
/// Parsing is identical either way ([`Catalog::from_raw`]).
struct RawData {
    /// `HelixModelDefs.bin` — the model table (names, categories, ids). Required.
    model_defs: Vec<u8>,
    /// `Helix.sym` — the device param-order table (the wire selector index). Required.
    symbols: Vec<u8>,
    /// `HelixControls.json` — enum/segmented control labels. Best-effort (empty → no labels).
    controls: Vec<u8>,
    /// `HX_ModelCatalog.json` — read only for its per-category colours, so the editor paints a
    /// block the shade HX Edit paints it. Best-effort (empty → the front end keeps its own palette).
    catalog_json: Vec<u8>,
    /// The `.models` files present, as `(filename, bytes)`. Best-effort (a missing file just
    /// drops that category's DSP loads / param ranges).
    models: Vec<(String, Vec<u8>)>,
}

/// [`Catalog::link_sync`]'s body: mark each group's three members in `params`.
fn link_sync_groups(params: &mut [EditorParam], groups: &[[String; 3]]) {
    for [tempo_sym, note_sym, governed_sym] in groups {
        let find = |sym: &str| params.iter().position(|p| p.name == sym);
        let (Some(tempo), Some(note), Some(governed)) =
            (find(tempo_sym), find(note_sym), find(governed_sym))
        else {
            continue;
        };
        for (i, role) in [
            (tempo, SyncRole::Tempo),
            (note, SyncRole::Note),
            (governed, SyncRole::Governed),
        ] {
            params[i].meta.sync = Some(SyncLink {
                role,
                tempo,
                note,
                governed,
            });
        }
    }
}

/// Pull every model's tempo-sync groups out of `HX_ModelCatalog.json` — see [`SyncLink`] for the
/// shape. Keyed by the model's `id`, which is its `.models` `symbolicID` (checked: every one of
/// the 106 sync-carrying ids is in the `.models` files). Best-effort like the colours: no file,
/// no groups, three rows.
fn sync_groups_from(bytes: &[u8]) -> std::collections::HashMap<String, Vec<[String; 3]>> {
    let mut out = std::collections::HashMap::new();
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return out;
    };
    fn walk(v: &serde_json::Value, out: &mut std::collections::HashMap<String, Vec<[String; 3]>>) {
        match v {
            serde_json::Value::Object(m) => {
                if let (
                    Some(serde_json::Value::String(id)),
                    Some(serde_json::Value::Array(params)),
                ) = (m.get("id"), m.get("params"))
                {
                    let groups: Vec<[String; 3]> = params
                        .iter()
                        .filter_map(|e| {
                            let list = e.as_array()?;
                            // Each entry is a one-key object, `{symbol: display-name-or-null}`.
                            let syms: Vec<&str> = list
                                .iter()
                                .filter_map(|o| o.as_object()?.keys().next().map(String::as_str))
                                .collect();
                            match syms.as_slice() {
                                [t, n, g]
                                    if t.starts_with("TempoSync")
                                        && n.starts_with("SyncSelect") =>
                                {
                                    Some([t.to_string(), n.to_string(), g.to_string()])
                                }
                                _ => None,
                            }
                        })
                        .collect();
                    if !groups.is_empty() {
                        out.insert(id.clone(), groups);
                    }
                }
                for x in m.values() {
                    walk(x, out);
                }
            }
            serde_json::Value::Array(a) => {
                for x in a {
                    walk(x, out);
                }
            }
            _ => {}
        }
    }
    walk(&v, &mut out);
    out
}

/// Pull category name → `0xRRGGBB` out of `HX_ModelCatalog.json`.
///
/// The file nests categories under several keys and repeats the shape for subcategories, so this
/// walks the whole tree and takes any object carrying both a `name` and a `color`. Colours are
/// `"0xRRGGBB"` strings, in mixed case.
///
/// Best-effort throughout: a missing or unparseable file yields an empty map and the front end
/// falls back to its own palette. This never fails a catalog load — the colours are cosmetic and
/// the editor's job is to work without the reference data at all.
fn category_colors_from(bytes: &[u8]) -> std::collections::HashMap<String, u32> {
    let mut out = std::collections::HashMap::new();
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return out;
    };
    fn walk(v: &serde_json::Value, out: &mut std::collections::HashMap<String, u32>) {
        match v {
            serde_json::Value::Object(m) => {
                if let (Some(serde_json::Value::String(name)), Some(serde_json::Value::String(c))) =
                    (m.get("name"), m.get("color"))
                    && let Some(hex) = c.strip_prefix("0x").or_else(|| c.strip_prefix("0X"))
                    && let Ok(rgb) = u32::from_str_radix(hex, 16)
                {
                    out.entry(name.clone()).or_insert(rgb);
                }
                for x in m.values() {
                    walk(x, out);
                }
            }
            serde_json::Value::Array(a) => {
                for x in a {
                    walk(x, out);
                }
            }
            _ => {}
        }
    }
    walk(&v, &mut out);
    out
}

/// The `.models` files the catalog reads for DSP loads, per-param ranges, and cab links.
/// `io.models` is in here for its params (input gate, output level/pan) and for the fixed
/// input/output/split/mixer nodes' own `load` figures — which the loads table carries but
/// [`EditorPreset::dsp_load`] does not sum; see [`DSP_CEILING`].
const MODEL_FILES: &[&str] = &[
    "amp.models",
    "cab.models",
    "cabmicirs.models",
    "cabmicirswithpan.models",
    "compressor.models",
    "delay.models",
    "distortion.models",
    "eq.models",
    "filter.models",
    "fixed.models",
    "gate.models",
    "io.models",
    "modulation.models",
    "pitch-synth.models",
    "preamp.models",
    "reverb.models",
    "sendreturn.models",
    "volumepan.models",
    "wah.models",
];

impl Catalog {
    /// Load the catalog for normal use: from the user's imported reference data
    /// ([`crate::data_dir`], written by `fretwire import-data`) when present, else the build-time
    /// embedded copy (only if the `bundled-data` feature is on). A release build without either
    /// returns an error pointing the user at the import step.
    pub fn load() -> crate::Result<Catalog> {
        Catalog::load_family(crate::import::HX)
    }

    /// [`Catalog::load`] for the device family serving `model_code` (a preset's key `7 -> 36`, or
    /// the handshake's identity string). The POD Go indexes its own symbol table, so decoding one
    /// of its presets against the HX data resolves every block to the wrong model — this is how a
    /// caller that knows which device it is talking to asks for the right table.
    pub fn load_for_model(model_code: &str) -> crate::Result<Catalog> {
        Catalog::load_family(crate::import::DataFamily::for_model_code(model_code))
    }

    fn load_family(family: crate::import::DataFamily) -> crate::Result<Catalog> {
        let dir = family.dir_under(&crate::data_dir());
        // The symbol table is the one file we can't do without, so its presence gates the data dir.
        if dir.join(family.symbols).is_file() {
            return Catalog::from_data_dir(&dir);
        }
        #[cfg(feature = "bundled-data")]
        {
            return Catalog::bundled();
        }
        #[cfg(not(feature = "bundled-data"))]
        {
            // Name the family, not just "reference data". Each device family reads its own vendor
            // files from its own directory, so a POD Go owner who has imported HX Edit has a data
            // dir full of files and still cannot decode their pedal — telling them there is "no
            // reference data" and pointing them back at the HX installer is wrong twice over.
            Err(crate::Error::MissingData(format!(
                "no {} reference data in {} — run `fretwire import-data <{} installer>` first",
                family.label,
                dir.display(),
                family.label
            )))
        }
    }

    /// Load the catalog at runtime from a directory of imported HX Edit reference files (see
    /// [`crate::data_dir`]). `HelixModelDefs.bin` and `Helix.sym` are required; the rest degrade
    /// gracefully if absent (params lose ranges/labels but still edit by index).
    pub fn from_data_dir(dir: &Path) -> crate::Result<Catalog> {
        // Which editor's files these are — POD Go Edit ships the same four formats under its own
        // names. Falls back to the HX spelling so the error below names the file people expect.
        let family = crate::import::DataFamily::detect(dir).unwrap_or(crate::import::HX);
        let require = |name: &str| -> crate::Result<Vec<u8>> {
            std::fs::read(dir.join(name)).map_err(|e| {
                crate::Error::MissingData(format!(
                    "{} in {} ({e}) — run `fretwire import-data <{} installer>`",
                    name,
                    dir.display(),
                    family.label
                ))
            })
        };
        let raw = RawData {
            model_defs: require(family.model_defs)?,
            symbols: require(family.symbols)?,
            controls: std::fs::read(dir.join(family.controls)).unwrap_or_default(),
            catalog_json: std::fs::read(dir.join(family.catalog_json)).unwrap_or_default(),
            models: MODEL_FILES
                .iter()
                .filter_map(|&n| std::fs::read(dir.join(n)).ok().map(|b| (n.to_string(), b)))
                .collect(),
        };
        Catalog::from_raw(&raw)
    }

    /// Load the catalog from the reference data embedded in the binary at build time. Only available
    /// with the `bundled-data` feature (off by default; requires a local, unshipped
    /// `crates/fretwire-data/data/` copy to compile). Prefer [`Catalog::load`] in application code.
    #[cfg(feature = "bundled-data")]
    pub fn bundled() -> crate::Result<Catalog> {
        macro_rules! model {
            ($f:literal) => {
                (
                    $f.to_string(),
                    include_bytes!(concat!("../../fretwire-data/data/", $f)).to_vec(),
                )
            };
        }
        let raw = RawData {
            model_defs: include_bytes!("../../fretwire-data/data/HelixModelDefs.bin").to_vec(),
            symbols: include_bytes!("../../fretwire-data/data/Helix.sym").to_vec(),
            controls: include_bytes!("../../fretwire-data/data/HelixControls.json").to_vec(),
            catalog_json: include_bytes!("../../fretwire-data/data/HX_ModelCatalog.json").to_vec(),
            models: vec![
                model!("amp.models"),
                model!("cab.models"),
                model!("cabmicirs.models"),
                model!("cabmicirswithpan.models"),
                model!("compressor.models"),
                model!("delay.models"),
                model!("distortion.models"),
                model!("eq.models"),
                model!("filter.models"),
                model!("fixed.models"),
                model!("gate.models"),
                model!("io.models"),
                model!("modulation.models"),
                model!("pitch-synth.models"),
                model!("preamp.models"),
                model!("reverb.models"),
                model!("sendreturn.models"),
                model!("volumepan.models"),
                model!("wah.models"),
            ],
        };
        Catalog::from_raw(&raw)
    }

    /// Parse a [`RawData`] set (however sourced) into a `Catalog`.
    fn from_raw(raw: &RawData) -> crate::Result<Catalog> {
        Ok(Catalog {
            models: ModelDefs::parse(&raw.model_defs).map_err(crate::Error::Data)?,
            symbols: DeviceSymbols::parse(&raw.symbols).map_err(crate::Error::Data)?,
            loads: loads_from(&raw.models),
            param_meta: param_meta_from(&raw.models, &raw.controls),
            cab_links: cab_links_from(&raw.models),
            category_colors: category_colors_from(&raw.catalog_json),
            sync_groups: sync_groups_from(&raw.catalog_json),
        })
    }

    /// A model's parameters at their **defaults**, named and described as a block of it would be
    /// — what a front end can show about a model before it is added, with no device. `index` is
    /// the `Helix.sym` index [`ModelChoice::index`] carries. `None` for an index the table does
    /// not have.
    ///
    /// Everything here is the same data a loaded block's params carry ([`ParamMeta`], the
    /// tempo-sync links); the values are `.models` defaults, or zero where a model gives none.
    /// The trailing extra a block sends beyond its symbol's list (`Trails`) is not listed — it is
    /// not in the symbol table, and it is not a model parameter.
    pub fn model_params(
        &self,
        index: i64,
    ) -> Option<(String, Option<&'static str>, Vec<EditorParam>)> {
        let (symbol, order) = self.symbols.by_index(usize::try_from(index).ok()?)?;
        let (base, variant) = split_variant(symbol);
        let (name, category) = self.resolve_name(Some(symbol));
        let meta = self.param_meta.get(base);
        let values: Vec<ParamValue> = order
            .iter()
            .map(|sym| {
                let m = meta.and_then(|m| m.get(sym));
                let default = m.and_then(|m| m.default).unwrap_or(0.0);
                match m.and_then(|m| m.value_type) {
                    Some(0) => ParamValue::Int(default.round() as i64),
                    Some(2) => ParamValue::Bool(default != 0.0),
                    _ => ParamValue::Float(default as f32),
                }
            })
            .collect();
        let mut params = name_params(&values, Some(order), meta, category);
        self.link_sync(&mut params, Some(base));
        Some((name, variant, params))
    }

    /// Mark the members of every tempo-sync group of `model` in `params` — see [`SyncLink`].
    /// A group whose three symbols are not all present in this block (a variant that dropped
    /// one, or no reference data) is left alone: three rows shown is the fallback, not a wrong
    /// fold.
    fn link_sync(&self, params: &mut [EditorParam], model: Option<&str>) {
        if let Some(groups) = model.and_then(|m| self.sync_groups.get(m)) {
            link_sync_groups(params, groups);
        }
    }

    /// The colour HX Edit paints category `id`, as `0xRRGGBB`.
    ///
    /// `None` when the reference data has not been imported — the editor runs without it, so the
    /// front end keeps a palette of its own for that case. Our synthetic categories resolve to the
    /// thing they are made of: `Amp+Cab` and `Cab (Mic+IR)` take Amp's and Cab's colours, which in
    /// Line 6's scheme are the same red anyway.
    pub fn category_color(&self, id: i64) -> Option<u32> {
        let name = category_name(id)?;
        let name = match name {
            "Amp+Cab" => "Amp",
            "Cab (Mic+IR)" => "Cab",
            other => other,
        };
        self.category_colors.get(name).copied()
    }

    /// The `Helix.sym` index of an exact device symbol, if present.
    fn symbol_index(&self, symbolic_id: &str) -> Option<i64> {
        (0..self.symbols.len())
            .find(|&i| {
                self.symbols
                    .by_index(i)
                    .is_some_and(|(s, _)| s == symbolic_id)
            })
            .map(|i| i as i64)
    }

    /// The block's DSP load (% of budget) for the given `Helix.sym` variant: the stereo cost for a
    /// `Stereo` block, else the mono cost — falling back to whichever the model defines.
    fn model_load(&self, symbolic_id: &str, variant: Option<&str>) -> Option<f64> {
        let (mono, stereo) = self.loads.get(symbolic_id)?;
        match variant {
            Some("Stereo") => stereo.or(*mono),
            _ => mono.or(*stereo),
        }
    }

    /// DSP load (% of budget) for a model given its **`Helix.sym` index** — the variant is taken from
    /// the symbol. For fit checks when swapping by raw index (the wire selector). `None` if the index
    /// is out of range or the model declares no load.
    pub fn model_load_by_index(&self, index: i64) -> Option<f64> {
        let (sym, _) = self.symbols.by_index(index as usize)?;
        let (base, variant) = split_variant(sym);
        self.model_load(base, variant)
    }

    /// A model's display name by its `Helix.sym` index (e.g. for labeling an add/swap in the edit
    /// history before the block exists to read it from).
    pub fn model_name_by_index(&self, index: i64) -> Option<String> {
        let (sym, _) = self.symbols.by_index(index as usize)?;
        let (base, _) = split_variant(sym);
        Some(self.resolve_name(Some(base)).0)
    }

    /// The distinct model categories present in the table that have a known name, as `(id, name)`,
    /// ordered by name. Drives the category selector in the model picker (so a block can be swapped
    /// to a model in a *different* category).
    pub fn categories(&self) -> Vec<(i64, &'static str)> {
        let mut seen = std::collections::BTreeSet::new();
        for idx in 0..self.symbols.len() {
            let Some((sym, _)) = self.symbols.by_index(idx) else {
                continue;
            };
            // Look the symbol up **as the table spells it**. The two editors disagree on whether a
            // `symbolicID` keeps its Mono/Stereo suffix — HX Edit's defs are keyed by the base
            // (344 of them) and POD Go Edit's by the full suffixed symbol (180, and not one by the
            // base). `id_by_symbolic_id` falls back from full to base, so the full symbol is the
            // form that finds an entry in either convention; passing the stripped base silently
            // dropped every suffixed POD Go model — the missing Wah and Reverb categories — and
            // eight HX ones (the DL4 legacy delays) besides. [issue #15]
            if let Some(id) = self.models.id_by_symbolic_id(sym)
                && let Some(cat) = self.models.category(id)
            {
                seen.insert(canonical_category(cat));
            }
        }
        // The synthetic Amp+Cab list exists whenever there are amps to pair.
        if seen.contains(&1) {
            seen.insert(CATEGORY_AMP_CAB);
        }
        let mut out: Vec<(i64, &'static str)> = seen
            .into_iter()
            .filter_map(|id| category_name(id).map(|n| (id, n)))
            .collect();
        out.sort_by(|a, b| a.1.cmp(b.1));
        out
    }

    /// Swap candidates in `category`: one entry per model, preferring the `variant` (Mono/Stereo) of
    /// the block being replaced, sorted by display name. Each `index` is the **`Helix.sym` index** to
    /// pass to `swap_model`. Built by scanning the device symbol table, resolving each symbol's base
    /// id to its model-table category, and keeping those in the requested category. `variant` is the
    /// current block's variant (amps carry `None`); when a model offers both, the matching one wins.
    pub fn models_in_category(&self, category: i64, variant: Option<&str>) -> Vec<ModelChoice> {
        // Synthetic Amp+Cab: the amp list, each entry pre-paired with its `amp.models`-suggested
        // cab (combined DSP cost). Amps with no cab link (none today) just fall out of the list.
        if category == CATEGORY_AMP_CAB {
            return self
                .models_in_category(1, variant)
                .into_iter()
                .filter_map(|mut c| {
                    let cab = self.cab_links.get(&c.symbolic_id)?;
                    c.default_paired_index = Some(self.symbol_index(cab)?);
                    if let Some(cab_load) = self.model_load(cab, None) {
                        c.dsp_load = Some(c.dsp_load.unwrap_or(0.0) + cab_load);
                    }
                    c.category = Some(CATEGORY_AMP_CAB);
                    Some(c)
                })
                .collect();
        }
        let mut chosen: std::collections::HashMap<String, ModelChoice> =
            std::collections::HashMap::new();
        for idx in 0..self.symbols.len() {
            let Some((sym, _)) = self.symbols.by_index(idx) else {
                continue;
            };
            let (base, var) = split_variant(sym);
            // The full symbol, not `base` — see the note in `categories`.
            let Some(id) = self.models.id_by_symbolic_id(sym) else {
                continue;
            };
            if self.models.category(id).map(canonical_category) != Some(category) {
                continue;
            }
            if is_dual_cab(base) {
                continue;
            }
            let name = self
                .models
                .name(id)
                .map(str::to_string)
                .unwrap_or_else(|| base.to_string());
            if name.is_empty() {
                continue;
            }
            let choice = ModelChoice {
                index: idx as i64,
                symbolic_id: base.to_string(),
                name,
                category: Some(category),
                variant: var,
                dsp_load: self.model_load(base, var),
                default_paired_index: None,
            };
            // Keep one per model; prefer the variant matching the block being replaced.
            match chosen.get(base) {
                Some(existing) if existing.variant == variant || var != variant => {}
                _ => {
                    chosen.insert(base.to_string(), choice);
                }
            }
        }
        // Deterministic order: by name, then symbolic id (so same-named models don't shuffle between
        // renders — the `HashMap` iteration order is otherwise arbitrary).
        let mut out: Vec<ModelChoice> = chosen.into_values().collect();
        out.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.symbolic_id.cmp(&b.symbolic_id))
        });

        // Disambiguate genuinely-distinct models that share a display name (e.g. the amp vs preamp
        // "EV Panama Red", both in this category) by appending their type token (`Amp`/`Preamp`/…).
        let mut i = 0;
        while i < out.len() {
            let mut j = i + 1;
            while j < out.len() && out[j].name == out[i].name {
                j += 1;
            }
            if j - i > 1 {
                for c in &mut out[i..j] {
                    c.name = format!("{} ({})", c.name, type_token(&c.symbolic_id));
                }
            }
            i = j;
        }
        out
    }

    /// Decode a reassembled preset stream into the editor model.
    pub fn load_preset(&self, stream: &[u8]) -> crate::Result<EditorPreset> {
        let ps = PresetStream::parse(stream).map_err(crate::Error::Data)?;
        let blocks: Vec<EditorBlock> = ps
            .loaded_blocks()
            .into_iter()
            .map(|b| self.build_block(b))
            .collect();
        let dsp_load = blocks.iter().filter_map(|b| b.dsp_load).sum();
        // One view per populated DSP. On the Stomp that's a single entry; the Helix Floor has two,
        // each with its own split/mixer/input/output nodes and its own A/B branch.
        let dsps = ps
            .dsps()
            .into_iter()
            .map(|d| {
                use fretwire_data::stream::slot_kind;
                let split = ps.dsp_is_split(d);
                // Split/mixer routing nodes are meaningful only on a parallel DSP; expose them as
                // editable blocks (they carry a model-ref + params like any block) for the routing
                // panel.
                let (split_node, mixer_node, split_pos, mixer_pos) = if split {
                    (
                        ps.dsp_structural_node(d, slot_kind::SPLIT)
                            .map(|n| self.build_block(n)),
                        ps.dsp_structural_node(d, slot_kind::MIXER)
                            .map(|n| self.build_block(n)),
                        ps.dsp_structural_node_pos(d, slot_kind::SPLIT),
                        ps.dsp_structural_node_pos(d, slot_kind::MIXER),
                    )
                } else {
                    (None, None, None, None)
                };
                // The fixed input (index 0) / output (index 9) nodes: no model-ref; identity is the
                // device symbol, params are the leading entries of its order (io.models carries the
                // meta). Edited with the ordinary set-value op on their slot [solid — input-gate
                // capture].
                DspView {
                    dsp: d,
                    split,
                    split_node,
                    mixer_node,
                    split_pos,
                    mixer_pos,
                    input_node: self.build_io_node(
                        &ps,
                        d,
                        0,
                        &["HelixStomp_AppDSPFlowInput", "P34_AppDSPFlowInput"],
                        "Input",
                    ),
                    output_node: self.build_io_node(
                        &ps,
                        d,
                        1,
                        &["HelixStomp_AppDSPFlowOutputMain", "P34_AppDSPFlowOutput"],
                        "Output",
                    ),
                    grid: ps.dsp_grid(d),
                }
            })
            .collect();
        Ok(EditorPreset {
            device_model: ps.device_model(),
            build_stamp: ps.build_stamp(),
            blocks,
            assignments: ps.assignments(),
            footswitch_count: ps.footswitch_layout().len(),
            active_snapshot: ps.snapshots().0,
            snapshot_names: ps.snapshots().1,
            dsp_load,
            current: None, // offline decode has no device to ask; `Session::read_preset` fills this
            dsps,
        })
    }

    /// Build the input/output node as an editable block: params named from the device symbol's
    /// order (wire index space) with io.models meta, then given their display names (the sym names
    /// are code-ish: `noiseGate`, `gain`).
    fn build_io_node(
        &self,
        ps: &PresetStream,
        dsp: usize,
        kind: i64,
        sym_candidates: &[&str],
        display: &str,
    ) -> Option<EditorBlock> {
        const DISPLAY_NAMES: &[(&str, &str)] = &[
            ("noiseGate", "Input Gate"),
            ("threshold", "Threshold"),
            ("decay", "Decay"),
            ("pan", "Pan"),
            ("gain", "Level"),
        ];
        // The node's device symbol is family-spelled (`HelixStomp_…` on the HX line, `P34_…` on
        // the POD Go), so probe the loaded table for whichever spelling it actually has —
        // otherwise the params come back unnamed (issue #15's blank In/Out labels).
        let sym = sym_candidates
            .iter()
            .find(|s| self.symbols.params(s).is_some())
            .copied()
            .unwrap_or(sym_candidates[0]);
        let b = ps.dsp_io_node(dsp, kind)?;
        // IO nodes (gate / level-pan) have no category and no trailing-extra quirk.
        let mut params = name_params(
            &b.params,
            self.symbols.params(sym),
            self.param_meta.get(sym),
            None,
        );
        for p in &mut params {
            if let Some((_, d)) = DISPLAY_NAMES.iter().find(|(k, _)| *k == p.name) {
                p.name = d.to_string();
            }
        }
        Some(EditorBlock {
            dsp: b.dsp,
            slot: b.slot,
            model_name: display.to_string(),
            user_label: None,
            custom_color: None,
            momentary: false,
            symbolic_id: Some(sym.to_string()),
            model_index: None,
            category: None,
            bypassed: None,
            variant: None,
            is_controller: false,
            footswitch: 0,
            row: 0,
            params,
            paired_model_name: None,
            paired_index: None,
            paired_symbolic_id: None,
            paired_category: None,
            paired_params: Vec::new(),
            dsp_load: None,
        })
    }

    fn build_block(&self, b: fretwire_data::stream::LoadedBlock) -> EditorBlock {
        // Identity comes from the Helix.sym index: it gives the exact device symbol (with the
        // Mono/Stereo variant) and that symbol's authoritative param order.
        let sym = b
            .model_index
            .and_then(|i| self.symbols.by_index(i as usize));
        let (symbolic_id, variant) = match sym {
            Some((s, _)) => {
                let (base, v) = split_variant(s);
                (Some(base.to_string()), v)
            }
            None => (None, None),
        };
        let (model_name, category) = self.resolve_name(sym.map(|(s, _)| s));
        let meta = symbolic_id.as_deref().and_then(|s| self.param_meta.get(s));
        let mut params = name_params(&b.params, sym.map(|(_, p)| p), meta, category);
        self.link_sync(&mut params, symbolic_id.as_deref());

        // Paired cab/IR (amp+cab blocks): resolve its name + name its param group too.
        let paired_sym = b
            .paired_index
            .and_then(|i| self.symbols.by_index(i as usize));
        let paired_variant = paired_sym.and_then(|(s, _)| split_variant(s).1);
        let paired_symbolic = paired_sym.map(|(s, _)| split_variant(s).0.to_string());
        let (paired_model_name, paired_category) = self.resolve_name(paired_sym.map(|(s, _)| s));

        // DSP load (% of budget): the block's own model, plus its paired cab/IR if fused. Controller
        // nodes aren't DSP blocks, so they cost nothing.
        let dsp_load = if b.node_kind == Some(2) {
            None
        } else {
            let block = symbolic_id
                .as_deref()
                .and_then(|s| self.model_load(s, variant));
            let cab = paired_symbolic
                .as_deref()
                .and_then(|s| self.model_load(s, paired_variant));
            match (block, cab) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
            }
        };

        EditorBlock {
            dsp: b.dsp,
            slot: b.slot,
            model_name,
            user_label: b.user_label,
            custom_color: b.custom_color,
            momentary: b.momentary,
            symbolic_id,
            model_index: b.model_index,
            category,
            bypassed: b.bypassed,
            variant,
            is_controller: b.node_kind == Some(2),
            footswitch: b.footswitch,
            row: b.row,
            params,
            paired_model_name: if b.paired_index.is_some() {
                Some(paired_model_name)
            } else {
                None
            },
            paired_index: b.paired_index,
            paired_params: name_params(
                &b.paired_params,
                paired_sym.map(|(_, p)| p),
                paired_symbolic
                    .as_deref()
                    .and_then(|s| self.param_meta.get(s)),
                paired_category,
            ),
            paired_symbolic_id: paired_symbolic,
            paired_category: if b.paired_index.is_some() {
                paired_category
            } else {
                None
            },
            dsp_load,
        }
    }

    /// Resolve a `symbolicID` to its display name + category via the model table, falling back to
    /// the symbol itself (then `"<unknown>"`) when the table has no matching entry.
    ///
    /// Pass the **full** device symbol, `Mono`/`Stereo` suffix and all: the two vendors disagree on
    /// which form their model table is keyed by, so only the suffixed symbol can be matched against
    /// either (see [`ModelDefs::id_by_symbolic_id`]). The display fallback still uses the base, to
    /// match the `symbolic_id` the block carries.
    fn resolve_name(&self, symbolic_id: Option<&str>) -> (String, Option<i64>) {
        let id = symbolic_id.and_then(|s| self.models.id_by_symbolic_id(s));
        let name = id
            .and_then(|id| self.models.name(id))
            .map(str::to_string)
            .or_else(|| symbolic_id.map(|s| split_variant(s).0.to_string()))
            .unwrap_or_else(|| "<unknown>".to_string());
        (
            name,
            id.and_then(|id| self.models.category(id))
                .map(canonical_category),
        )
    }
}

/// Build the `symbolicID` → (mono load, stereo load) table from the given `.models` files. Only
/// models that declare a load are kept. Parse errors on a single file are skipped (best-effort —
/// the meter degrades to "unknown" for those models rather than failing the whole catalog).
fn loads_from(
    models: &[(String, Vec<u8>)],
) -> std::collections::HashMap<String, (Option<f64>, Option<f64>)> {
    let mut map = std::collections::HashMap::new();
    for (_, bytes) in models {
        let Ok(mf) = fretwire_data::models::ModelFile::from_slice(bytes) else {
            continue;
        };
        for m in mf.models {
            if m.load.is_some() || m.load_stereo.is_some() {
                map.insert(m.symbolic_id, (m.load, m.load_stereo));
            }
        }
    }
    map
}

/// Build the model `symbolicID` → (param `symbolicID` → [`ParamMeta`]) table from the given
/// `.models` files, using `controls` (`HelixControls.json`) for enum/segmented labels. Keeps each
/// model's per-param range and display type so the editor renders the right control with the right
/// bounds. Best-effort: a file that fails to parse is skipped.
fn param_meta_from(
    models: &[(String, Vec<u8>)],
    controls: &[u8],
) -> std::collections::HashMap<String, std::collections::HashMap<String, ParamMeta>> {
    let discrete = discrete_control_labels(controls);
    let segmented = segmented_float_controls(controls);
    let numeric = numeric_control_formats(controls);
    let mut map: std::collections::HashMap<String, std::collections::HashMap<String, ParamMeta>> =
        std::collections::HashMap::new();
    for (_, bytes) in models {
        let Ok(mf) = fretwire_data::models::ModelFile::from_slice(bytes) else {
            continue;
        };
        for m in mf.models {
            let params: std::collections::HashMap<String, ParamMeta> = m
                .params
                .iter()
                .map(|p| {
                    // An integer-enum param (valueType 0) gets its dropdown labels from the discrete
                    // control named by its displayType.
                    let enum_labels = if p.value_type == Some(0) {
                        p.display_type
                            .as_deref()
                            .and_then(|dt| discrete.get(dt))
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    // A float whose control is segmented gets discrete stops (cab mic Angle: 0/45).
                    let stops = if p.value_type == Some(1) {
                        p.display_type
                            .as_deref()
                            .and_then(|dt| segmented.get(dt))
                            .and_then(|ctl| segment_stops(p.min_f64(), p.max_f64(), ctl))
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    // Continuous floats get their unit-bearing display recipe. Enums and switches
                    // read as labels, and a segmented float shows its stop labels instead.
                    let control = (p.value_type == Some(1))
                        .then(|| p.display_type.as_deref().and_then(|dt| numeric.get(dt)))
                        .flatten();
                    let format = control.map(|ctl| ctl.resolved(p.min_f64(), p.max_f64()));
                    let step = control
                        .zip(format.as_ref())
                        .and_then(|(ctl, fmt)| ctl.raw_step(fmt));
                    let meta = ParamMeta {
                        // Per block, not per model — it names param indices. See `link_sync`.
                        sync: None,
                        // Only when it actually differs, so `display_name()` can fall through to
                        // the symbol and the common case costs no allocation.
                        label: p.name.clone().filter(|n| *n != p.symbolic_id),
                        min: p.min_f64(),
                        max: p.max_f64(),
                        display_type: p.display_type.clone(),
                        value_type: p.value_type,
                        enum_labels,
                        stops,
                        format,
                        step,
                        default: p.default_f64(),
                    };
                    (p.symbolic_id.clone(), meta)
                })
                .collect();
            // Key by the id as written *and* by its variant-stripped base. Blocks look meta up by
            // the base (`load_preset` runs the device symbol through `split_variant` first), so a
            // model the reference data only ever names in its suffixed form — the eight legacy DL4
            // delays are the whole set — would otherwise resolve to no meta at all, and a param
            // with no range falls back to a span far wider than the device accepts.
            //
            // The alias never overwrites: an exact entry is the better answer wherever both exist.
            if let (base, Some(_)) = split_variant(&m.symbolic_id)
                && !map.contains_key(base)
            {
                map.insert(base.to_string(), params.clone());
            }
            map.insert(m.symbolic_id, params);
        }
    }

    // The parallel routing nodes (split types + mixer/join) aren't in the `.models` files, so their
    // float params have no range → they'd be read-only. Give them best-effort ranges here so the
    // mixer A/B levels/pans and the split params are adjustable. Param names match the `Helix.sym`
    // order (`name_params` keys meta by name). LIVE: ranges are estimates pending a range capture.
    //
    // These being estimates matters more than it looks: the device does **not** clamp what it is
    // sent. An earlier note here assumed it did, and that an off estimate could therefore only
    // mis-scale the slider rather than the value — that is false. An out-of-range integer hung a
    // Helix Floor hard enough to drop it off USB. [solid — 2026-07-30 Floor session]
    // `Session::clamp_param` now bounds every write by whatever range lands here, so an estimate
    // that is too *narrow* costs reach, while one too *wide* is what carries risk. Prefer narrow.
    let flow = |pairs: &[(&str, f64, f64)]| -> std::collections::HashMap<String, ParamMeta> {
        pairs
            .iter()
            .map(|&(n, lo, hi)| {
                (
                    n.to_string(),
                    ParamMeta {
                        min: Some(lo),
                        max: Some(hi),
                        ..ParamMeta::default()
                    },
                )
            })
            .collect()
    };
    map.insert(
        "HD2_AppDSPFlowJoin".into(),
        flow(&[
            ("A Level", -60.0, 12.0),
            ("A Pan", 0.0, 1.0),
            ("B Level", -60.0, 12.0),
            ("B Pan", 0.0, 1.0),
            ("Level", -60.0, 12.0),
        ]),
    );
    map.insert(
        "HD2_AppDSPFlowSplitY".into(),
        flow(&[("BalanceA", 0.0, 1.0), ("BalanceB", 0.0, 1.0)]),
    );
    map.insert(
        "HD2_AppDSPFlowSplitAB".into(),
        flow(&[("RouteTo", 0.0, 1.0)]),
    );
    map.insert(
        "HD2_AppDSPFlowSplitXOver".into(),
        flow(&[("Frequency", 100.0, 8000.0)]),
    );
    map.insert(
        "HD2_AppDSPFlowSplitDyn".into(),
        flow(&[
            ("Threshold", -60.0, 0.0),
            ("Attack", 0.0, 1.0),
            ("Decay", 0.0, 1.0),
        ]),
    );
    map
}

/// Amp `symbolicID` → suggested cab `symbolicID`, from `amp.models`. Prefers `ircablink` (the
/// mic+IR cab engine current firmware pairs) over the legacy `cablink`. Only amp models carry
/// these fields, so scanning `amp.models` is sufficient.
fn cab_links_from(models: &[(String, Vec<u8>)]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Some((_, amps)) = models.iter().find(|(n, _)| n == "amp.models") else {
        return out;
    };
    let Ok(mf) = fretwire_data::models::ModelFile::from_slice(amps) else {
        return out;
    };
    for m in mf.models {
        if let Some(cab) = m.ircablink.or(m.cablink) {
            out.insert(m.symbolic_id, cab);
        }
    }
    out
}

/// Controls marked `controlType: "segmented"` but **not** `isDiscrete` — floats HX Edit renders as
/// a few fixed positions rather than a knob. `displayToWidgetScale` implies the stop spacing (the
/// widget spans 0..1, so stop count = value-span × scale + 1) and `format` is a printf-ish label
/// template (`"%.0f deg"`). Only `CabMicIrs_Angle` qualifies in today's data, but this reads the
/// shape from the shipped file rather than hardcoding the param.
fn segmented_float_controls(controls: &[u8]) -> std::collections::HashMap<String, (f64, String)> {
    let mut out = std::collections::HashMap::new();
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(controls) else {
        return out;
    };
    let Some(obj) = root.as_object() else {
        return out;
    };
    for (name, ctrl) in obj {
        if ctrl.get("controlType").and_then(serde_json::Value::as_str) != Some("segmented")
            || ctrl.get("isDiscrete").and_then(serde_json::Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(scale) = ctrl
            .get("displayToWidgetScale")
            .and_then(serde_json::Value::as_f64)
        else {
            continue;
        };
        if scale > 0.0 {
            let fmt = ctrl
                .get("format")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .into();
            out.insert(name.clone(), (scale, fmt));
        }
    }
    out
}

/// The discrete stops of a segmented float param, evenly spaced over its range (Angle 0–45 with
/// scale 1/45 → 2 stops: 0 and 45). `None` when the range/scale don't describe a sane segment
/// count — the param then falls back to a slider.
fn segment_stops(
    min: Option<f64>,
    max: Option<f64>,
    (scale, fmt): &(f64, String),
) -> Option<Vec<SegStop>> {
    let (min, max) = (min?, max?);
    let span = max - min;
    if span <= 0.0 {
        return None;
    }
    let n = (span * scale).round() as i64 + 1;
    if !(2..=8).contains(&n) {
        return None;
    }
    Some(
        (0..n)
            .map(|i| {
                let value = min + span * i as f64 / (n - 1) as f64;
                SegStop {
                    value,
                    label: seg_label(fmt, value),
                }
            })
            .collect(),
    )
}

/// Render a stop label from the control's printf-ish `format` (`"%.0f deg"` → `"0 deg"`). Only the
/// `%.Nf` form appears in the data; anything else falls back to the bare number.
fn seg_label(fmt: &str, v: f64) -> String {
    if let Some(pos) = fmt.find("%.") {
        let rest = &fmt[pos + 2..];
        if let Some(fpos) = rest.find('f')
            && let Ok(prec) = rest[..fpos].parse::<usize>()
        {
            return format!("{}{:.prec$}{}", &fmt[..pos], v, &rest[fpos + 1..]);
        }
    }
    format!("{v}")
}

/// How a numeric parameter should be shown. The device stores DSP values — a delay time is
/// `1.3728`, a mix is `0.5` — and HX Edit displays them scaled and carrying a unit ("1.373 s",
/// "50 %"). `HelixControls.json` holds that recipe, keyed by the param's `displayType`.
///
/// This is not cosmetic. The tester spent part of a session on an Adriatic Delay reading `1.3728`,
/// couldn't tell what it meant, and nearly filed it as "the delay isn't working" — it was a 1.4
/// second delay time, which "1.373 s" would have said outright.
#[derive(Debug, Clone, PartialEq)]
pub struct NumFormat {
    /// `dspToDisplayScale`: stored value × this = display units (seconds → ms, 0–1 → percent).
    /// For a control that gives its display span as `minimumValue`/`maximumValue` instead, this is
    /// derived from the param's own range — see [`display_affine`].
    pub scale: f64,
    /// Added after `scale`. Non-zero only for the bipolar controls, whose display span is centred
    /// on zero while the stored value runs 0..1 (pan: ×200 − 100, so 0.5 lands on 0 = "Center").
    pub offset: f64,
    /// Range rules in file order; the first whose bounds contain the scaled value wins. A control
    /// with one unranged format has a single rule spanning everything.
    pub rules: Vec<FormatRule>,
}

/// One range of a [`NumFormat`] — the file splits a control by magnitude so that, say, a delay
/// reads in ms up to a second and in seconds past it.
#[derive(Debug, Clone, PartialEq)]
pub struct FormatRule {
    pub lo: f64,
    pub hi: f64,
    /// `unitsMultiplier`: applied on top of `scale` for this range only (ms → s past 1000).
    pub mult: f64,
    /// printf-ish template — `formatUnits` where the file has one, so the unit comes with it.
    pub template: String,
}

impl NumFormat {
    /// Render `raw` (the stored value) the way HX Edit would. `None` only if there are no rules.
    pub fn display(&self, raw: f64) -> Option<String> {
        let scaled = raw * self.scale + self.offset;
        let rule = self
            .rules
            .iter()
            .find(|r| scaled >= r.lo && scaled < r.hi)
            .or_else(|| self.rules.last())?;
        Some(printf_f(&rule.template, scaled * rule.mult))
    }
}

/// Substitute a float into a printf-ish template: `%[+][.N]f`, with `%%` for a literal percent.
/// Only these forms appear in the reference data; anything else is passed through untouched (some
/// templates are pure text, e.g. `blend`'s "Equal").
fn printf_f(template: &str, v: f64) -> String {
    let mut out = String::with_capacity(template.len() + 8);
    let b = template.as_bytes();
    let mut i = 0;
    let mut used = false;
    while i < b.len() {
        if b[i] != b'%' {
            out.push(b[i] as char);
            i += 1;
            continue;
        }
        if i + 1 < b.len() && b[i + 1] == b'%' {
            out.push('%');
            i += 2;
            continue;
        }
        // %[+][.N]f — anything that doesn't match is copied verbatim.
        let mut j = i + 1;
        let plus = j < b.len() && b[j] == b'+';
        if plus {
            j += 1;
        }
        let mut prec = 0usize;
        if j < b.len() && b[j] == b'.' {
            j += 1;
            let s = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            prec = template[s..j].parse().unwrap_or(0);
        }
        if j < b.len() && b[j] == b'f' && !used {
            if plus && v >= 0.0 {
                out.push('+');
            }
            out.push_str(&format!("{v:.prec$}"));
            used = true;
            i = j + 1;
        } else {
            out.push('%');
            i += 1;
        }
    }
    out
}

/// A continuous control as `HelixControls.json` describes it: the display recipe, plus the display
/// span for the controls that state one instead of a `dspToDisplayScale`.
#[derive(Debug, Clone)]
struct ControlFormat {
    fmt: NumFormat,
    /// `step.fine`, in display units, when the file states a single one — see [`ParamMeta::step`].
    fine_step: Option<f64>,
    /// `minimumValue`/`maximumValue`, kept only when the file left `dspToDisplayScale` out and gave
    /// this instead. Five controls do, all bipolar: `pan`, `blend`, `tilt`, `split_ab_route_to`,
    /// `split_balance`. Everywhere else the two forms would fight, so the explicit scale wins.
    display_span: Option<(f64, f64)>,
}

impl ControlFormat {
    /// The recipe as it applies to one param. A control that states its span rather than a scale
    /// can only be finished here, where the param's own stored range is known — the same control
    /// serves params with different ranges.
    fn resolved(&self, min: Option<f64>, max: Option<f64>) -> NumFormat {
        let mut fmt = self.fmt.clone();
        if let Some(span) = self.display_span
            && let (Some(lo), Some(hi)) = (min, max)
            && let Some((scale, offset)) = display_affine((lo, hi), span)
        {
            fmt.scale = scale;
            fmt.offset = offset;
        }
        fmt
    }

    /// [`ParamMeta::step`]: the fine increment in stored units. Needs the *resolved* format, since
    /// the file's step is in display units and pan's scale is only known per-param.
    fn raw_step(&self, fmt: &NumFormat) -> Option<f64> {
        let step = self.fine_step? / fmt.scale;
        step.is_finite().then_some(step).filter(|s| *s > 0.0)
    }
}

/// `scale`/`offset` mapping a param's stored range onto the display span its control declares, so
/// that `lo → dlo` and `hi → dhi`.
///
/// For the five bipolar controls this is the entire recipe: pan stores 0..1 and displays -100..100,
/// i.e. ×200 − 100, which puts the stored 0.5 on 0 — "Center". Without it the stored value went
/// through the format rules unscaled, so everything from centre to hard right read "Center" and the
/// rest "Right 0"/"Right 1"; the same wrongness is why the slider looked like it moved in jumps.
/// [solid — the pedal showed L100 for a slider fretwire labelled "Center", 2026-08-20 XL report;
/// reproduced on an HX Stomp, whose synth `PanVoice1` reads "Right 0" at the stored 0.5]
///
/// `None` on a stored range that is degenerate or not finite — there is no mapping to make.
fn display_affine((lo, hi): (f64, f64), (dlo, dhi): (f64, f64)) -> Option<(f64, f64)> {
    let span = hi - lo;
    if !span.is_finite() || span == 0.0 || !dlo.is_finite() || !dhi.is_finite() {
        return None;
    }
    let scale = (dhi - dlo) / span;
    Some((scale, dlo - lo * scale))
}

/// Parse `HelixControls.json` into control name → [`ControlFormat`] for the **continuous** controls
/// (the discrete ones are label lists — see [`discrete_control_labels`]). Resolves the `alias`
/// indirection the file uses for its 58 range-specialised aliases (`time_ms_20_1800` → `time_ms`).
fn numeric_control_formats(controls: &[u8]) -> std::collections::HashMap<String, ControlFormat> {
    let mut out = std::collections::HashMap::new();
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(controls) else {
        return out;
    };
    let Some(obj) = root.as_object() else {
        return out;
    };
    for name in obj.keys() {
        // Follow `alias` hops, bounded so a cycle in the data can't hang the import.
        let mut ctrl = &obj[name];
        for _ in 0..8 {
            match ctrl.get("alias").and_then(serde_json::Value::as_str) {
                Some(target) => match obj.get(target) {
                    Some(next) => ctrl = next,
                    None => break,
                },
                None => break,
            }
        }
        if ctrl.get("isDiscrete").and_then(serde_json::Value::as_bool) == Some(true) {
            continue;
        }
        // A control gives its display mapping one way or the other, never both: a multiplier, or
        // the span the stored range maps onto. `display_affine` finishes the second form once the
        // param is known.
        let explicit_scale = ctrl
            .get("dspToDisplayScale")
            .and_then(serde_json::Value::as_f64);
        let bound = |k| ctrl.get(k).and_then(serde_json::Value::as_f64);
        // `step` is either `{fine, coarse}` or an array of those bracketed by range. Only the
        // single-object form gives one increment that holds across the param.
        let fine_step = ctrl
            .get("step")
            .and_then(serde_json::Value::as_object)
            .and_then(|st| st.get("fine"))
            .and_then(serde_json::Value::as_f64);
        let display_span = match explicit_scale {
            Some(_) => None,
            None => bound("minimumValue").zip(bound("maximumValue")),
        };
        let units = |v: &serde_json::Value| {
            v.get("formatUnits")
                .or_else(|| v.get("format"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        let mut rules = Vec::new();
        match ctrl.get("format") {
            Some(serde_json::Value::Array(arr)) => {
                for e in arr {
                    let Some(t) = units(e) else { continue };
                    rules.push(FormatRule {
                        lo: e
                            .get("lowerBound")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(f64::NEG_INFINITY),
                        hi: e
                            .get("upperBound")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(f64::INFINITY),
                        mult: e
                            .get("unitsMultiplier")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(1.0),
                        template: t,
                    });
                }
            }
            // A scalar `format`, or none at all but a bare `formatUnits`: one rule for everything.
            _ => {
                if let Some(t) = units(ctrl) {
                    rules.push(FormatRule {
                        lo: f64::NEG_INFINITY,
                        hi: f64::INFINITY,
                        mult: 1.0,
                        template: t,
                    });
                }
            }
        }
        if !rules.is_empty() {
            out.insert(
                name.clone(),
                ControlFormat {
                    fmt: NumFormat {
                        scale: explicit_scale.unwrap_or(1.0),
                        offset: 0.0,
                        rules,
                    },
                    fine_step,
                    display_span,
                },
            );
        }
    }
    out
}

/// Parse `HelixControls.json` into a map of **discrete** control name → ordered option labels (the
/// `format` string array of every entry marked `isDiscrete`). This is how enum params (the cab `Mic`,
/// reverb room sizes, etc.) get their dropdown labels: a param's `displayType` names its control
/// here. Best-effort: returns an empty map if the bundled file can't be parsed.
fn discrete_control_labels(controls: &[u8]) -> std::collections::HashMap<String, Vec<String>> {
    let mut out = std::collections::HashMap::new();
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(controls) else {
        return out;
    };
    let Some(obj) = root.as_object() else {
        return out;
    };
    for (name, ctrl) in obj {
        if ctrl.get("isDiscrete").and_then(serde_json::Value::as_bool) != Some(true) {
            continue;
        }
        // `format` is the discrete label list (an array of strings). Some controls instead carry
        // per-range format objects — those aren't enum label lists, so skip non-string arrays.
        if let Some(arr) = ctrl.get("format").and_then(serde_json::Value::as_array) {
            let labels: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if labels.len() == arr.len() && !labels.is_empty() {
                out.insert(name.clone(), labels);
            }
        }
    }
    out
}

/// The leading "type" word of a `symbolicID` (`HD2_PreampEVPanamaRed` → `"Preamp"`,
/// `HD2_AmpEVPanamaRed` → `"Amp"`): the body after the vendor prefix up to the first lower→upper
/// camelCase boundary. Used to disambiguate models that share a display name. Falls back to the
/// whole body when there's no clean boundary.
fn type_token(symbolic_id: &str) -> &str {
    let body = symbolic_id
        .split_once('_')
        .map(|(_, b)| b)
        .unwrap_or(symbolic_id);
    let bytes = body.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i].is_ascii_uppercase() && bytes[i - 1].is_ascii_lowercase() {
            return &body[..i];
        }
    }
    body
}

/// Split a device symbol into its `symbolicID` base and `Mono`/`Stereo` variant (some models —
/// e.g. amps — carry no suffix, giving `None`).
/// Whether `symbol` is the **dual** twin of a Cab (Mic+IR) model — HX Edit's `Cab › Dual`
/// subcategory, a two-cab block with per-cab pan.
///
/// The 46 `HD2_CabMicIr_*WithPan` symbols each shadow a plain `HD2_CabMicIr_*` single cab of the
/// same display name, so a flat category listing shows every cab twice. The twins are not
/// interchangeable: they are a different block type, and the pedal **refuses an in-place swap** to
/// one — device code `-306`, and `-21` in some states. Selecting a duplicate therefore did
/// nothing and the block snapped back to what it was. Editing a dual cab is not supported yet
/// (it needs two model refs and the pan params), so keep them out of the swap list rather than
/// offering a choice that cannot be taken. Name resolution is deliberately left alone — a preset
/// that already contains one must still read back with its own name and params.
///
/// [solid — 2026-08-01: reproduced on the HX Stomp, every dual twin refused; matches the
/// field report and the shipped `HX_ModelCatalog.json` `Cab › Dual` grouping.]
fn is_dual_cab(symbol: &str) -> bool {
    symbol.starts_with("HD2_CabMicIr_") && symbol.ends_with("WithPan")
}

fn split_variant(symbol: &str) -> (&str, Option<&'static str>) {
    if let Some(base) = symbol.strip_suffix("Mono") {
        (base, Some("Mono"))
    } else if let Some(base) = symbol.strip_suffix("Stereo") {
        (base, Some("Stereo"))
    } else {
        (symbol, None)
    }
}

/// Name the lone trailing value a model sends beyond its symbol's listed params, chosen by category:
/// time-based fx (delay/reverb) append a `Trails` on/off switch, while legacy (non-`CabMicIr_*`) cabs
/// append a **mic-index** value the symbol omits. Labeling the cab's mic index `Trails` was a bug
/// (seen on the Floor's `HD2_Cab2x12MailC12Q`, and on any Stomp preset using an old-style cab).
fn trailing_extra_name(category: Option<i64>) -> &'static str {
    match category {
        // 2 = Cab, 19 = Cab (Mic+IR). Native CabMicIr cabs list the mic, so they never hit this
        // branch; legacy cabs don't, and their trailing value is the mic index.
        Some(2) | Some(19) => "Mic",
        _ => "Trails",
    }
}

/// Name a value vector against an ordered name list. Values past the named list become `#i`, except
/// a lone trailing extra the symbol doesn't list — named by [`trailing_extra_name`] from the model's
/// `category`.
fn name_params(
    values: &[ParamValue],
    order: Option<&[String]>,
    meta: Option<&std::collections::HashMap<String, ParamMeta>>,
    category: Option<i64>,
) -> Vec<EditorParam> {
    let names = order.unwrap_or(&[]);
    values
        .iter()
        .enumerate()
        .map(|(i, &value)| {
            // A value past the symbol's list is addressed through the extras table rather than by
            // param index. A block has exactly one such value in every case seen so far — that is
            // the branch that names it below — so its extras index is 0. If a block ever sends two,
            // we have no evidence for what the second one's index is: leave it unaddressable rather
            // than guess. With no reference data at all there is no list to be past, so nothing is
            // an extra and every param keeps the ordinary path.
            let is_extra = order.is_some() && i >= names.len();
            let lone_extra = i == names.len() && values.len() == names.len() + 1;
            let extra_index = (is_extra && lone_extra).then_some(0);
            let settable = !is_extra || extra_index.is_some();
            let name = names.get(i).cloned().unwrap_or_else(|| {
                if i == names.len() && values.len() == names.len() + 1 {
                    trailing_extra_name(category).to_string()
                } else {
                    format!("#{i}")
                }
            });
            // The param's name is its `.models` `symbolicID`, so the range/widget metadata is a
            // direct lookup. Unknown params (e.g. the trailing `Trails`) get default (empty) meta.
            let meta = meta.and_then(|m| m.get(&name)).cloned().unwrap_or_default();
            EditorParam {
                index: i,
                name,
                value,
                meta,
                settable,
                extra_index,
            }
        })
        .collect()
}

// The catalog's sync grouping, on a hand-written excerpt — no reference data needed.
#[cfg(test)]
mod sync_group_tests {
    use super::*;

    const EXCERPT: &str = r#"{"categories":[{"name":"Mod","subcategories":[{"models":[
        {"id":"HD2_TremoloOpticalTrem","name":"Optical Trem","params":[
            [{"TempoSync1":null},{"SyncSelect1":"Note Sync"},{"Speed":null}],{"Intensity":null},{"Level":null}]},
        {"id":"HD2_Chorus70sChorus","name":"70s Chorus","params":[
            [{"TempoSync1":null},{"SyncSelect1":"Note Sync (Chorus)"},{"ChorusIntensity":null}],
            [{"TempoSync2":null},{"SyncSelect2":"Note Sync (Vibrato)"},{"VibratoRate":null}],{"Mix":null}]},
        {"id":"HD2_GateHardGate","name":"Hard Gate","params":[{"Threshold":null},{"Decay":null}]}
    ]}]}]}"#;

    #[test]
    fn nested_lists_are_the_groups() {
        let g = sync_groups_from(EXCERPT.as_bytes());
        assert_eq!(g.len(), 2, "a model with no nested list has no entry");
        assert_eq!(
            g["HD2_TremoloOpticalTrem"],
            vec![[
                "TempoSync1".to_string(),
                "SyncSelect1".into(),
                "Speed".into()
            ]]
        );
        assert_eq!(g["HD2_Chorus70sChorus"].len(), 2);
        assert_eq!(g["HD2_Chorus70sChorus"][1][2], "VibratoRate");
        assert!(sync_groups_from(b"not json").is_empty());
    }

    #[test]
    fn link_marks_all_three_members_from_any_side() {
        let groups = sync_groups_from(EXCERPT.as_bytes());
        let trem = &groups["HD2_TremoloOpticalTrem"];
        let order: Vec<String> = ["Speed", "Intensity", "TempoSync1", "SyncSelect1", "Level"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let values = vec![ParamValue::Float(0.5); 5];
        let mut params = name_params(&values, Some(&order), None, Some(6));
        link_sync_groups(&mut params, trem);
        let link = |i: usize| params[i].meta.sync.expect("linked");
        assert_eq!(link(0).role, SyncRole::Governed);
        assert_eq!(link(2).role, SyncRole::Tempo);
        assert_eq!(link(3).role, SyncRole::Note);
        for i in [0, 2, 3] {
            assert_eq!((link(i).tempo, link(i).note, link(i).governed), (2, 3, 0));
        }
        assert!(params[1].meta.sync.is_none() && params[4].meta.sync.is_none());

        // A block missing one of the three (no such variant exists, but a foreign preset could
        // carry one) keeps three plain rows rather than a fold that points at nothing.
        let short: Vec<String> = ["Speed", "TempoSync1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut params = name_params(&values[..2], Some(&short), None, Some(6));
        link_sync_groups(&mut params, trem);
        assert!(params.iter().all(|p| p.meta.sync.is_none()));
    }
}

// Pure naming tests — no reference data needed, so they always run.
#[cfg(test)]
mod trailing_extra_tests {
    use super::{name_params, trailing_extra_name};
    use fretwire_data::stream::ParamValue;

    #[test]
    fn cab_categories_name_the_trailing_extra_mic_not_trails() {
        assert_eq!(trailing_extra_name(Some(2)), "Mic"); // Cab
        assert_eq!(trailing_extra_name(Some(19)), "Mic"); // Cab (Mic+IR)
        assert_eq!(trailing_extra_name(Some(10)), "Trails"); // Reverb
        assert_eq!(trailing_extra_name(Some(9)), "Trails"); // Delay
        assert_eq!(trailing_extra_name(None), "Trails");
    }

    #[test]
    fn legacy_cab_trailing_value_is_named_mic() {
        // A legacy cab: the symbol lists 5 params but the device sends 6 — the extra is the mic
        // index. It must be "Mic", not "Trails".
        let order: Vec<String> = ["Distance", "LowCut", "HighCut", "EarlyReflections", "Level"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let values = vec![ParamValue::Float(0.0); 6];
        let params = name_params(&values, Some(&order), None, Some(2));
        assert_eq!(params.last().unwrap().name, "Mic");

        // Same shape, but a reverb (category 10) keeps the Trails name.
        let params = name_params(&values, Some(&order), None, Some(10));
        assert_eq!(params.last().unwrap().name, "Trails");
    }

    /// A value past the symbol's list has no param index, so it is addressed through the extras
    /// table instead (target key 29 `false`, key 28 = the extras index). A block sends exactly one
    /// such value, so that index is 0 — the wire form HX Edit uses to toggle Trails.
    #[test]
    fn the_trailing_extra_is_addressed_as_an_extra() {
        let order: Vec<String> = ["Time", "Feedback", "Mix"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let values = vec![ParamValue::Float(0.0); 4];
        let params = name_params(&values, Some(&order), None, Some(9));
        assert!(
            params[..3]
                .iter()
                .all(|p| p.settable && p.extra_index.is_none()),
            "listed params keep the ordinary param-index path"
        );
        assert_eq!(params[3].name, "Trails");
        assert_eq!(params[3].extra_index, Some(0));
        assert!(params[3].settable);
    }

    /// Two values past the list is a shape we have never seen and have no wire evidence for. Leave
    /// those unaddressable rather than guessing an extras index for each.
    #[test]
    fn several_values_past_the_list_stay_unsettable() {
        let order: Vec<String> = ["Time", "Mix"].iter().map(|s| s.to_string()).collect();
        let values = vec![ParamValue::Float(0.0); 4];
        let params = name_params(&values, Some(&order), None, Some(9));
        assert!(
            params[2..]
                .iter()
                .all(|p| !p.settable && p.extra_index.is_none())
        );
    }

    /// …but with no reference data there is no symbol list to check against, and marking every
    /// param read-only would make a clean clone useless. Fail open.
    #[test]
    fn without_a_name_list_every_param_stays_settable() {
        let values = vec![ParamValue::Float(0.0); 4];
        let params = name_params(&values, None, None, Some(9));
        assert!(params.iter().all(|p| p.settable));
    }
}

// These tests validate against the (unshipped) Line 6 reference data, so they only compile when a
// local copy is present — `have_bundled_data`, set by build.rs. They load it via `from_data_dir`
// (always available), not `bundled()` (which needs the `bundled-data` feature), so they run under a
// plain `cargo test` on a dev machine.
#[cfg(all(test, have_bundled_data))]
mod tests {
    use super::*;

    /// The Line 6 reference data from the `fretwire import-data` cache (dev/test only; not shipped).
    fn dev_catalog() -> Catalog {
        Catalog::from_data_dir(&crate::data_dir())
            .expect("load reference data (run `fretwire import-data`)")
    }

    /// A model described before it exists on the pedal: its params at their defaults, with the
    /// same names, ranges and sync links a block of it would carry.
    #[test]
    fn model_params_describe_a_model_offline() {
        let cat = dev_catalog();
        let simple = cat
            .models_in_category(9, Some("Mono"))
            .into_iter()
            .find(|m| m.name == "Simple Delay")
            .expect("a Simple Delay in the delay category");
        let (name, variant, params) = cat.model_params(simple.index).expect("described");
        assert_eq!(name, "Simple Delay");
        assert_eq!(variant, Some("Mono"));
        let time = params
            .iter()
            .find(|p| p.name == "Time")
            .expect("a Time param");
        assert!(time.meta.max.is_some(), "ranges come from .models");
        assert_eq!(time.meta.sync.map(|s| s.role), Some(SyncRole::Governed));
        let note = params
            .iter()
            .find(|p| p.name == "SyncSelect1")
            .expect("the note value");
        assert_eq!(note.display_name(), "Note Sync");
        assert!(
            !note.meta.enum_labels.is_empty(),
            "labels come from HelixControls"
        );
        assert!(cat.model_params(-1).is_none() && cat.model_params(1 << 20).is_none());
    }

    /// The shipped catalog's grouping, end to end: every `.models` model that carries a sync pair
    /// gets a group, and the group names a param the model actually has.
    #[test]
    fn every_sync_pair_in_the_catalog_governs_a_real_param() {
        let cat = dev_catalog();
        assert!(
            cat.sync_groups.len() >= 100,
            "{} models with groups",
            cat.sync_groups.len()
        );
        for (model, groups) in &cat.sync_groups {
            let Some(meta) = cat.param_meta.get(model) else {
                panic!("{model} has sync groups but no .models entry");
            };
            for [t, n, g] in groups {
                for sym in [t, n, g] {
                    assert!(
                        meta.contains_key(sym),
                        "{model}: {sym} is not one of its params"
                    );
                }
            }
        }
        // The one everybody knows: a Simple Delay's TempoSync1/SyncSelect1 take over Time.
        let simple = cat
            .sync_groups
            .iter()
            .find(|(m, _)| m.contains("SimpleDelay"))
            .map(|(_, g)| g.clone())
            .expect("a simple delay");
        assert_eq!(simple[0][2], "Time");
    }

    /// Snapshot names are 1-based on the pedal's screen and 0-based in the preset — the off-by-one
    /// that would rename snapshot 1 to `SNAPSHOT 0` and leave `clear_preset` renaming a snapshot
    /// that was already right. The strings are the ones an untouched HX Stomp preset carries.
    #[test]
    fn default_snapshot_names_are_one_based() {
        let names: Vec<String> = (0..3).map(default_snapshot_name).collect();
        assert_eq!(names, ["SNAPSHOT 1", "SNAPSHOT 2", "SNAPSHOT 3"]);
    }

    /// The case that sent the tester chasing a delay he thought was broken: stored `1.3728` is a
    /// 1.4-second delay time, and the raw number said none of that. Also pins the two shapes the
    /// recipe has to handle — a range switch with a `unitsMultiplier` (ms → s past 1000) and the
    /// plain scaled percent.
    #[test]
    fn a_delay_time_reads_in_seconds_not_raw_dsp_units() {
        let meta = dev_catalog().param_meta;
        let delay = meta
            .get("HD2_DelayAdriaticDelay")
            .expect("bundled delay model present");

        let time = delay.get("Time").expect("delay has a Time param");
        let f = time.format.as_ref().expect("Time has a display format");
        assert_eq!(f.display(1.3728).as_deref(), Some("1.373 s"));
        // Under a second it stays in milliseconds, and under 10 ms it gains a decimal.
        assert_eq!(f.display(0.4).as_deref(), Some("400 ms"));
        assert_eq!(f.display(0.005).as_deref(), Some("5.0 ms"));

        let mix = delay.get("Mix").expect("delay has a Mix param");
        let f = mix.format.as_ref().expect("Mix has a display format");
        assert_eq!(f.display(0.5).as_deref(), Some("50 %"));

        // A switch is read as a label, so it must not carry a numeric recipe.
        let mic = meta["HD2_CabMicIr_2x12JazzRivet"]["Mic"].clone();
        assert!(mic.format.is_none());
    }

    #[test]
    fn a_volume_param_keeps_its_sign_and_unit() {
        let meta = dev_catalog().param_meta;
        let lvl = meta["HD2_DelayAdriaticDelay"]["Level"]
            .format
            .clone()
            .expect("Level has a display format");
        assert_eq!(lvl.display(0.0).as_deref(), Some("+0.0 dB"));
        assert_eq!(lvl.display(-4.89).as_deref(), Some("-4.9 dB"));
    }

    /// The offset rule behind [`ParamMeta::enum_base`], checked against the whole shipped catalog
    /// rather than the one control that exposed it. Every labelled enum whose range doesn't start
    /// at 0 has exactly one label per value in `min..=max` — so `value - min` is the index, and a
    /// data drop that broke that would fail here instead of silently shifting a dropdown by one.
    #[test]
    fn a_labelled_enum_covers_its_declared_range() {
        let meta = dev_catalog().param_meta;
        let mut offset_enums = 0;
        for (model, params) in &meta {
            for (name, m) in params {
                if m.enum_labels.is_empty() || m.enum_base() == 0 {
                    continue;
                }
                let (min, max) = (m.min.unwrap(), m.max.unwrap());
                assert_eq!(
                    m.enum_labels.len(),
                    (max - min) as usize + 1,
                    "{model}.{name} ({:?}) labels {min}..={max}",
                    m.display_type
                );
                assert_eq!(m.enum_label(min as i64), Some(m.enum_labels[0].as_str()));
                offset_enums += 1;
            }
        }
        // sync_note alone is on 100+ models; a zero here would mean the check stopped checking.
        assert!(offset_enums > 100, "only {offset_enums} offset enums seen");
    }

    #[test]
    fn cab_mic_param_is_a_12_option_enum() {
        let meta = dev_catalog().param_meta;
        let cab = meta
            .get("HD2_CabMicIr_2x12JazzRivet")
            .expect("bundled cab model present");
        let mic = cab.get("Mic").expect("cab has a Mic param");
        assert_eq!(mic.value_type, Some(0)); // integer enum
        assert_eq!(mic.enum_labels.len(), 12); // 12 mics (matches cab_mic_order.txt)
        assert_eq!(mic.enum_labels[0], "57 Dynamic");
        assert_eq!(mic.enum_labels[11], "67 Cond");
        // A continuous knob on the same cab must NOT be treated as an enum.
        let distance = cab.get("Distance").expect("cab has a Distance param");
        assert_eq!(distance.value_type, Some(1));
        assert!(distance.enum_labels.is_empty());
    }

    #[test]
    fn cab_mic_angle_is_a_two_stop_segmented_float() {
        let meta = dev_catalog().param_meta;
        let cab = meta
            .get("HD2_CabMicIr_2x12JazzRivet")
            .expect("bundled cab model present");
        let angle = cab.get("Angle").expect("cab has an Angle param");
        assert_eq!(angle.value_type, Some(1)); // still a float on the wire
        assert_eq!(angle.stops.len(), 2); // …but HX Edit renders exactly two positions
        assert_eq!(angle.stops[0].value, 0.0);
        assert_eq!(angle.stops[1].value, 45.0);
        assert_eq!(angle.stops[0].label, "0 deg");
        assert_eq!(angle.stops[1].label, "45 deg");
        // Continuous floats must stay sliders.
        assert!(cab.get("Distance").unwrap().stops.is_empty());
    }

    /// A model whose `.models` id carries a `Mono`/`Stereo` suffix must still be reachable under
    /// the **variant-stripped** id, because that is the only form a block ever asks for:
    /// `load_preset` runs the device symbol through [`split_variant`] before looking meta up.
    ///
    /// The eight legacy DL4 delays are the only models in the reference data where the two forms
    /// differ, and the miss was not cosmetic — with no meta the editor has no range, and its
    /// fallback span (0..=127) let `Heads 1-2` (a 0..=3 enum) be set to 77, which hung the pedal
    /// hard enough to drop it off USB. [solid — 2026-07-30 Floor session, `Massif`]
    #[test]
    fn legacy_dl4_ranges_survive_the_variant_suffix() {
        let meta = dev_catalog().param_meta;
        let dl4 = meta
            .get("HD2_DL4Multihead")
            .expect("legacy DL4 reachable under the stripped id the device sends");
        let heads = dl4.get("Heads 1-2").expect("DL4 has a Heads 1-2 param");
        assert_eq!(heads.value_type, Some(0));
        assert_eq!(heads.min, Some(0.0));
        assert_eq!(
            heads.max,
            Some(3.0),
            "out-of-range writes here wedge the DSP"
        );
        // The suffixed form stays reachable too — nothing that already worked may regress.
        assert!(meta.contains_key("HD2_DL4MultiheadStereo"));
    }

    // Only meaningful when the embed exists to compare against.
    #[cfg(feature = "bundled-data")]
    #[test]
    fn from_data_dir_matches_bundled() {
        // The runtime loader (used by release builds without embedded data) must produce the same
        // catalog as the build-time embed. Point it at the crate's dev copy of the reference files.
        let disk = Catalog::from_data_dir(&crate::data_dir()).expect("load catalog from data dir");
        let embed = Catalog::bundled().expect("bundled catalog");
        assert_eq!(disk.symbols.len(), embed.symbols.len(), "symbol count");
        assert_eq!(disk.loads.len(), embed.loads.len(), "DSP load table size");
        assert_eq!(
            disk.cab_links.len(),
            embed.cab_links.len(),
            "cab link table size"
        );
        assert_eq!(
            disk.param_meta.len(),
            embed.param_meta.len(),
            "param meta model count"
        );
        // A representative deep value: the cab Mic enum resolved from HelixControls.json on disk.
        let disk_mic = &disk.param_meta["HD2_CabMicIr_2x12JazzRivet"]["Mic"].enum_labels;
        let embed_mic = &embed.param_meta["HD2_CabMicIr_2x12JazzRivet"]["Mic"].enum_labels;
        assert_eq!(disk_mic, embed_mic, "cab Mic enum labels");
    }

    #[test]
    fn mic_ir_cabs_are_listed_with_us_super() {
        let cat = dev_catalog();
        assert!(
            cat.categories()
                .iter()
                .any(|&(id, name)| id == 19 && name == "Cab (Mic+IR)")
        );
        let cabs = cat.models_in_category(19, None);
        assert!(
            cabs.iter().any(|c| c.name.contains("4x10 US Super")),
            "4x10 US Super missing from category 19: {:?}",
            cabs.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn amp_cab_category_pairs_amps_with_their_linked_cab() {
        let cat = dev_catalog();
        assert!(
            cat.categories()
                .iter()
                .any(|&(id, name)| id == CATEGORY_AMP_CAB && name == "Amp+Cab")
        );
        let combos = cat.models_in_category(CATEGORY_AMP_CAB, None);
        assert!(!combos.is_empty());
        // Every entry carries a resolvable paired cab, and its load includes the cab's cost.
        for c in &combos {
            let cab_idx = c
                .default_paired_index
                .expect("amp+cab entry has a paired cab") as usize;
            assert!(
                cat.symbols.by_index(cab_idx).is_some(),
                "{}: bad cab index",
                c.name
            );
        }
        // Spot-check the German Mahadeva → 4x12 Uber V30 link from amp.models.
        let mahadeva = combos
            .iter()
            .find(|c| c.symbolic_id == "HD2_AmpGermanMahadeva")
            .expect("Mahadeva in Amp+Cab");
        let (cab_sym, _) = cat
            .symbols
            .by_index(mahadeva.default_paired_index.unwrap() as usize)
            .unwrap();
        assert_eq!(cab_sym, "HD2_CabMicIr_4x12UberV30");
        let plain = cat.models_in_category(1, None);
        let plain_mahadeva = plain
            .iter()
            .find(|c| c.symbolic_id == "HD2_AmpGermanMahadeva")
            .unwrap();
        assert!(mahadeva.dsp_load.unwrap() > plain_mahadeva.dsp_load.unwrap());
    }
}

#[cfg(test)]
mod display_format_tests {
    use super::{ControlFormat, FormatRule, NumFormat, display_affine};

    /// The `pan` control verbatim from `HelixControls.json`: a display span, no `dspToDisplayScale`,
    /// and three rules that read the sign of the *display* value.
    fn pan_control() -> ControlFormat {
        let rule = |lo: f64, hi: f64, mult: f64, t: &str| FormatRule {
            lo,
            hi,
            mult,
            template: t.to_string(),
        };
        ControlFormat {
            fmt: NumFormat {
                scale: 1.0,
                offset: 0.0,
                rules: vec![
                    rule(-99999.0, -0.5, -1.0, "Left %.0f"),
                    rule(-0.5, 0.5, 1.0, "Center"),
                    rule(0.5, 999999.0, 1.0, "Right %.0f"),
                ],
            },
            fine_step: Some(1.0),
            display_span: Some((-100.0, 100.0)),
        }
    }

    #[test]
    fn a_stored_range_maps_onto_the_display_span_its_control_declares() {
        // Pan: 0..1 stored, -100..100 shown.
        assert_eq!(
            display_affine((0.0, 1.0), (-100.0, 100.0)),
            Some((200.0, -100.0))
        );
        // The identity case a scale-bearing control would describe as 1.0.
        assert_eq!(display_affine((0.0, 100.0), (0.0, 100.0)), Some((1.0, 0.0)));
    }

    #[test]
    fn a_degenerate_stored_range_has_no_mapping() {
        assert_eq!(display_affine((0.5, 0.5), (-100.0, 100.0)), None);
        assert_eq!(display_affine((0.0, f64::INFINITY), (-100.0, 100.0)), None);
    }

    /// The bug the XL report caught: unresolved, every stored value from centre to hard right reads
    /// "Center" and the rest "Right 0"/"Right 1" — which is also why the slider looked like it
    /// jumped rather than swept.
    #[test]
    fn pan_reads_as_the_pedal_shows_it_only_once_the_span_is_resolved() {
        let unresolved = pan_control().fmt;
        assert_eq!(unresolved.display(0.0).as_deref(), Some("Center"));
        assert_eq!(unresolved.display(0.5).as_deref(), Some("Right 0"));

        let fmt = pan_control().resolved(Some(0.0), Some(1.0));
        assert_eq!(fmt.display(0.0).as_deref(), Some("Left 100"));
        assert_eq!(fmt.display(0.25).as_deref(), Some("Left 50"));
        assert_eq!(fmt.display(0.5).as_deref(), Some("Center"));
        assert_eq!(fmt.display(0.75).as_deref(), Some("Right 50"));
        assert_eq!(fmt.display(1.0).as_deref(), Some("Right 100"));
        // The two the HX Stomp's synth block reports today.
        assert_eq!(fmt.display(0.478).as_deref(), Some("Left 4"));
        assert_eq!(fmt.display(0.544).as_deref(), Some("Right 9"));
    }

    /// A param with no range of its own can't be mapped, so the control is left as the file gave it
    /// rather than silently scaled by a guess.
    #[test]
    fn a_param_without_a_range_keeps_the_unresolved_recipe() {
        let ctl = pan_control();
        assert_eq!(ctl.resolved(None, None), ctl.fmt);
        assert_eq!(ctl.resolved(Some(0.0), None), ctl.fmt);
    }

    /// One wheel notch should move one display unit. Pan's stored range is 200 display units wide,
    /// so that is 0.005 — half the blanket 1/100th-of-range the panel used, which is why it scrolled
    /// two units a notch.
    #[test]
    fn the_fine_step_is_converted_into_stored_units() {
        let ctl = pan_control();
        let fmt = ctl.resolved(Some(0.0), Some(1.0));
        assert_eq!(ctl.raw_step(&fmt), Some(0.005));
        // One notch from centre is one display unit, in both directions.
        assert_eq!(fmt.display(0.5 + 0.005).as_deref(), Some("Right 1"));
        assert_eq!(fmt.display(0.5 - 0.005).as_deref(), Some("Left 1"));
    }

    /// A range-switched step is a curve, not a number — the panel keeps its own fallback rather
    /// than be handed a value that is wrong across most of the range.
    #[test]
    fn a_control_without_a_single_step_offers_none() {
        let mut ctl = pan_control();
        ctl.fine_step = None;
        assert_eq!(ctl.raw_step(&ctl.resolved(Some(0.0), Some(1.0))), None);
    }

    /// A control that states a scale keeps it: `display_span` is only read when there was none, so
    /// the two forms can never fight.
    #[test]
    fn an_explicit_scale_is_not_overridden() {
        let mut ctl = pan_control();
        ctl.fmt.scale = 100.0;
        ctl.display_span = None;
        assert_eq!(ctl.resolved(Some(0.0), Some(1.0)).scale, 100.0);
    }
}
