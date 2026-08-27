//! Convert HX Edit's **`tone` JSON** into the wire preset the device exchanges.
//!
//! A `tone` tree is what an `.hlx` preset file carries and what sits inside every slot of an
//! `.hxb` device backup ([`crate::hxb`]). It is the *host* side of the same preset the device
//! streams as integer-keyed MessagePack ([`crate::stream`]) — the same blocks, in a different
//! encoding: models named by symbol rather than by index, parameters keyed by name rather than
//! ordered, and structure spelled out as `@path`/`@position` rather than as a slot number.
//!
//! Going host → device is what a **restore** needs. Reading a `.hxb` has worked since 2026-07-26
//! but restoring from one did not, because nothing turned a `tone` object back into a blob.
//!
//! # The mapping  [solid — 2026-08-25]
//!
//! Reconciled block-for-block against a single preset held in **both** forms: a contributor's
//! Helix Floor backup (`FACTORY 1` slot 45, "Pull Me Under") and a wire dump of that same preset
//! read off that same unit fifteen hours later. All 15 blocks, both DSPs, both topologies, and
//! every one of the 106 parameter values agree. `tests/tone_to_wire.rs` is that comparison.
//!
//! | tone | wire |
//! |---|---|
//! | `@model` + `@stereo` | device symbol → its index in `Helix.sym` (block `24 → 25`) |
//! | named parameters | `11 → 4`, ordered by that symbol's `Helix.sym` parameter list |
//! | `@path`, `@position` | slot index = `@path × 10 + @position + 1` |
//! | `@enabled` | content key `10` |
//! | `@mic`, `@trails` | one extra value appended past the symbol's parameters |
//! | `@type` | content key `9`, the block class |
//! | `footswitch.dspN.blockM` | preset key `3 → 8`, with its labels and LED colours |
//! | `snapshotN` | preset key `10`, including the per-slot bypass matrix |
//!
//! The two counts beside the value vector say exactly this: `11 → 3` is the **symbol's** parameter
//! count and `11 → 2` the number of values **stored**, and they differ precisely on the blocks that
//! carry a trailing `@mic` or `@trails`.
//!
//! # What this writes, and what it leaves alone
//!
//! Conversion is an **overlay onto a preset the device itself wrote** — the caller supplies a
//! [`PresetStream`] read from the target unit, and this replaces the parts the `tone` tree
//! determines. Everything else keeps the device's own bytes: preset key `5` (86 fields, most
//! undecoded), key `2`, the device-info stamp at key `7`, and the input/output nodes, whose stored
//! parameter vector is a ragged prefix of its symbol's list that two samples cannot pin down.
//!
//! The **controller-assignment table** (key `4`) is rebuilt from the `tone`'s `controller`
//! section — the donor's own rows address blocks by slot and every slot has just been rewritten —
//! together with the snapshots' per-controller values (`10 → 10[n] → 2`), which index off its
//! places, and the type-2 footswitch-layout row a switch-sourced assignment also owns. The one
//! source kind still skipped is MIDI, whose row carries a CC number no `tone` we hold shows —
//! see [`apply_controllers`].
//!
//! That is the conservative direction for the same reason the rest of this crate takes it: a blob
//! is written to flash, and a field we guessed wrong is not visible until a musician's preset
//! sounds different. [`Conversion::not_carried`] names every such omission rather than leaving the
//! caller to assume the conversion was total.
//!
//! For the same reason a block whose class we have never seen on the wire is a **refusal**, not a
//! guess — see [`block_class`].
//!
//! # `@cab`: paired blocks  [solid — measured on an HX Stomp, 2026-08-26]
//!
//! An **Amp+Cab** block (`@type` 3) and a **dual cab** (`@type` 4) each carry `@cab: "cab0"`,
//! which names a *sibling entry of the same `dspN` object* — not a model. That entry holds the
//! second model: its own `@model`, `@mic` and parameters. On the wire the pair is one block, with
//! the sibling's index at `24 → 26`, `24 → 23` true, and its parameters in the **second** bank at
//! `12 → 4` — laid out exactly as that model's bank `11` would be (the same symbol order, the same
//! trailing `@mic` for a legacy cab, the same dropped `IrData` for a new one).
//!
//! This section used to be a refusal, on the belief that the tone and the wire named a paired cab
//! in **different symbol families** (`HD2_Cab…` against `HD2_CabMicIr_…`) with an unknown
//! correspondence. Pairing every combination on a live HX Stomp dissolved it: **both families are
//! ordinary `Helix.sym` entries and the wire stores whichever one the preset actually uses.** The
//! one dump we held (`HD2_CabMicIr_1x12USDeluxe`) was a new-family cab because it was built on
//! 3.80 firmware; the backup's `HD2_Cab…` siblings are the legacy family, which the device still
//! runs. There is no mapping — a tone's cab `@model` resolves like any other model.
//!
//! What the tone spells inconsistently is the parameter *names*: the same legacy cab stores
//! `HighCut` in one preset and `High Cut` in another (142 against 26 in one real backup, plus a
//! `Low Cut` and an `Early Reflections`), tracking whichever HX Edit era wrote it. Lookups
//! therefore match names with spaces stripped and case folded, not byte-for-byte.

use serde_json::{Map as JsonMap, Value as Json};

use crate::stream::{
    DSP_GROUP_KEYS, DSP_SLOT_STRIDE, PresetStream, map_get, map_get_mut, set_map_key, slot_kind,
};
use crate::symbols::DeviceSymbols;
use crate::{Error, Result};
use rmpv::Value;

/// Distance between the two rows in the slot array — row A starts at 1, row B at 11.
///
/// The 20-slot array is a fixed topology — `[0 = input, 1..=8 = row A, 9 = output, 10 = split,
/// 11..=18 = row B, 19 = mixer]` — so a `tone` block's `@path`/`@position` addresses it directly.
/// Same layout [`PresetStream::dsp_grid`] draws.
const ROW_STRIDE: usize = 10;
/// Columns per row. `@position` is 0-based and slot 1 is column 1.
const ROW_WIDTH: usize = 8;
/// One of the two structural nodes a DSP's slot array always holds, split or mixer.
///
/// Each is addressed the same three ways: where it sits in the slot array, what the `tone` calls
/// it, and which letter of the topology string says whether it is a real branch point.
#[derive(Debug, Clone, Copy)]
struct Node {
    /// Fixed index in the 20-slot array.
    index: usize,
    /// The `tone`'s key for it inside `dspN`.
    tone_key: &'static str,
    /// The letter in `global.@topologyN` that means this node is active on this DSP.
    marker: char,
}

impl Node {
    const SPLIT: Node = Node {
        index: 10,
        tone_key: "split",
        marker: 'S',
    };
    const MIXER: Node = Node {
        index: 19,
        tone_key: "join",
        marker: 'J',
    };
}

/// What a conversion carried, and what it did not.
///
/// `not_carried` exists because this conversion is deliberately partial: it is one line per part of
/// the `tone` tree that was left at the donor's value, so a caller can say so rather than implying
/// the preset came across whole.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conversion {
    /// Effect blocks written, across all DSPs.
    pub blocks: usize,
    /// Snapshots written.
    pub snapshots: usize,
    /// Footswitch bindings written.
    pub footswitches: usize,
    /// Parameter-controller assignments written (preset key `4`).
    pub controllers: usize,
    /// Parts of the `tone` tree this does not yet write, one human-readable line each.
    pub not_carried: Vec<String>,
}

/// A `tone` block's `@type` + its resolved model(s) → the wire's block-class byte (content key `9`).
///
/// Every value here was read off a device-authored preset. The class is not a function of `@type`
/// alone: the device tells the two cab families apart (a `HD2_CabMicIr_…` cab carries IR data, a
/// legacy `HD2_Cab…` does not), and a dual IR is its own class. So the caller passes the resolved
/// **device symbol** of the block's model, and of its `@cab` sibling where one exists.
///
/// | `@type` | class | evidence |
/// |---:|---:|---|
/// | 0 | 1 | every non-amp, non-cab, non-delay block in all six fixtures |
/// | 1 | 17 | fixtures (amp, `26` = −1); preamp measured identically on the Stomp |
/// | 2 | 15 legacy / **31** new | 15 from fixtures; both swept live on the Stomp |
/// | 3 | **18** legacy cab / 33 new | 33 from fixtures — every amp in them pairs a new cab; 18 measured live, refuting the old unconditional 33 |
/// | 4 | **16** legacy / **32** new | cab paired with cab, measured live both ways |
/// | 5 | **19** mono / **21** dual | measured live (`ImpulseResponse1024Mono`, `…1024DualStereo`) |
/// | 7 | 8 | every delay and reverb, and exactly the blocks carrying a trailing `@trails` |
/// | 8 | **23** | measured live (`Synth3NoteGeneratorMono`) |
///
/// [solid — fixtures 2026-08-25; every bolded value swept on a live HX Stomp 2026-08-26 by
/// swapping slot 1 through the combinations and reading back what the device wrote. The pattern —
/// 15/16/17/18/19 consecutive, +16 where the new cab engine is involved — is descriptive, not
/// assumed: each cell is its own measurement.]
///
/// `@type` 6 (looper) is a different slot shape entirely (kind 7, class 22) and is built by
/// [`encode_looper`], not through this table.
pub fn block_class(tone_type: i64, symbol: &str, paired_symbol: Option<&str>) -> Option<i64> {
    let new_cab = |s: &str| s.starts_with("HD2_CabMicIr");
    match tone_type {
        0 => Some(1),
        1 => Some(17),
        2 => Some(if new_cab(symbol) { 31 } else { 15 }),
        // An amp+cab with no cab sibling has never been observed; its class is the sibling's call.
        3 => paired_symbol.map(|cab| if new_cab(cab) { 33 } else { 18 }),
        4 => Some(if new_cab(symbol) { 32 } else { 16 }),
        5 => Some(if symbol.contains("Dual") { 21 } else { 19 }),
        7 => Some(8),
        8 => Some(23),
        _ => None,
    }
}

/// A `tone`'s `global.@topologyN` string → the DSP group's split-type integer (preset key `21`).
///
/// The string spells out what the DSP's signal path does: `A` is one path, `S` opens a split, `AB`
/// is two paths running, `J` closes them. So `SABJ` splits and rejoins on this DSP, while a bracket
/// spanning both DSPs of a Floor reads `SAB` on the first and `ABJ` on the second.
///
/// | string | key 21 | evidence |
/// |---|---:|---|
/// | `A` | 0 | "Sultans" — `("A","A")` against a dump reading `[0, 0]` |
/// | `SABJ` | 1 | both Stomp parallel fixtures read `1`, and a Stomp has one DSP, so a preset that splits at all must split and rejoin on it |
/// | `SAB` | 2 | "Pull Me Under" DSP1 |
/// | `ABJ` | 3 | "Pull Me Under" DSP2 |
///
/// `AB` — a DSP the paths merely pass through, on four presets in the backup — has no dump behind
/// it and so gets no number. The three-DSP-spanning shapes it appears in are rare and a wrong split
/// type is a preset whose two paths don't go where its owner put them.
///
/// [solid for `A`/`SAB`/`ABJ`, each read off a preset held in both forms; `SABJ` is one inference
/// from the Stomp's single DSP — 2026-08-25]
pub fn topology_code(topology: &str) -> Option<i64> {
    match topology {
        "A" => Some(0),
        "SABJ" => Some(1),
        "SAB" => Some(2),
        "ABJ" => Some(3),
        _ => None,
    }
}

/// Write a `tone` tree's blocks and split topology onto a preset the device wrote.
///
/// **The caller must confirm that `donor` and `tone` come from the same device** before calling.
/// Slot geometry and `Helix.sym` indices are per-device, so overlaying a Floor tone onto a Stomp
/// preset yields a blob that parses cleanly and means something else. The check is not made here
/// because the two id spaces live in different crates — a `tone` names its device numerically
/// (`0x0021_0001`) and a stream names it by model code (`"P21"`), and only `fretwire_protocol`'s
/// `Device` table relates them. `fretwire_core::restore` is where that check belongs.
///
/// Re-serialize the result with [`PresetStream::to_blob`]; the offset table is rebuilt there.
pub fn apply_tone(
    donor: &mut PresetStream,
    tone: &JsonMap<String, Json>,
    syms: &DeviceSymbols,
) -> Result<Conversion> {
    let mut out = Conversion::default();
    for (dsp, &group_key) in DSP_GROUP_KEYS.iter().enumerate() {
        let tone_key = format!("dsp{dsp}");
        let Some(tone_dsp) = tone.get(&tone_key).and_then(Json::as_object) else {
            continue;
        };
        // A DSP the donor doesn't have (an HX Stomp's key 1 is nil) can't take blocks.
        if !matches!(map_get(&donor.preset, group_key), Some(Value::Map(_))) {
            if tone_dsp.keys().any(|k| k.starts_with("block")) {
                out.not_carried.push(format!(
                    "{tone_key}: the target preset has no second DSP, so its blocks were dropped"
                ));
            }
            continue;
        }

        clear_blocks(donor, group_key);
        for (name, block) in tone_dsp {
            if !name.starts_with("block") {
                continue;
            }
            let index = slot_index(block).ok_or_else(|| {
                Error::Stream(format!("{tone_key}.{name}: no usable @path/@position"))
            })?;
            let encoded = encode_block(block, tone_dsp, syms)
                .map_err(|e| Error::Stream(format!("{tone_key}.{name}: {e}")))?;
            write_slot(donor, group_key, index, encoded)?;
            out.blocks += 1;
        }

        let topology = dsp_topology(tone, dsp);
        apply_node(
            donor,
            group_key,
            tone_dsp,
            Node::SPLIT,
            topology,
            syms,
            &mut out,
        );
        apply_node(
            donor,
            group_key,
            tone_dsp,
            Node::MIXER,
            topology,
            syms,
            &mut out,
        );
        apply_topology(donor, group_key, topology, dsp, &mut out);
    }

    let (placed, controller_switch_rows) = apply_controllers(donor, tone, syms, &mut out);
    apply_snapshots(donor, tone, &placed, &mut out);
    apply_footswitches(donor, tone, controller_switch_rows, &mut out);
    note_omissions(tone, &mut out);
    Ok(out)
}

/// Preset key `3 → 8` — the **footswitch layout**, with its custom labels and LED colours.
///
/// The array has one position per switch (FS*n* is position *n* − 1) and each position holds the
/// bindings on that switch, so a switch with two blocks on it is a two-element array. The `tone`
/// keys the same thing by block, at `footswitch.dspN.blockM`, and names the switch inside the entry
/// as `@fs_index` — so this is a transpose, then a slot lookup through the same `@path`/`@position`
/// mapping the blocks use.
///
/// | tone | wire |
/// |---|---|
/// | `@fs_index` | the array position, minus one |
/// | `@fs_primary` | first on the switch; `10` is the entry's position there |
/// | `@fs_label` | `11 → 5`, NUL-terminated |
/// | `@fs_ledcolor` | `11 → 6`, `0xRRGGBB` |
/// | `@fs_enabled` | `11 → 7` |
/// | the block's slot | `11 → 8`, the wire slot |
/// | `@fs_momentary` | `12` |
/// | `@fs_customlabel` | `14`, NUL-terminated, with `13` saying whether there is one |
///
/// [solid — 2026-08-25, all ten bindings of the oracle preset, including the one switch carrying
/// two blocks and the two whose `@fs_enabled` is false]
///
/// **`@fs_enabled` is not "is it bound".** A binding with it false is still in the array, with
/// `11 → 7` false — it is a block assigned to a switch that is not currently answering to it. The
/// oracle has two, and dropping them would quietly unbind blocks their owner had put there.
///
/// This writes only **bypass** bindings (`11 → 0` = 1). A parameter controller is a type-2 entry
/// here *and* a row in key `4`, and this does not write key `4` — see [`clear_controllers`] — so
/// writing it in one table and not the other would leave the two disagreeing.
fn apply_footswitches(
    donor: &mut PresetStream,
    tone: &JsonMap<String, Json>,
    controller_rows: Vec<(usize, bool, Value)>,
    out: &mut Conversion,
) {
    let placement = block_placement(tone);
    let slot_of = |dsp: usize, name: &str| {
        placement
            .iter()
            .find(|((d, n), _)| *d == dsp && n == name)
            .map(|(_, slot)| *slot)
    };

    // Collect (switch, position-on-switch, entry) before touching the donor: the tone is keyed by
    // block and the wire by switch, so everything has to be in hand to order a switch's entries.
    // A switch-sourced parameter controller is a type-2 row in this same array — those arrive
    // ready-built from [`apply_controllers`].
    let mut bindings: Vec<(usize, bool, Value)> = controller_rows;
    let mut unplaceable = 0usize;
    for dsp in 0..DSP_GROUP_KEYS.len() {
        let Some(switches) = tone
            .get("footswitch")
            .and_then(|f| f.get(format!("dsp{dsp}")))
            .and_then(Json::as_object)
        else {
            continue;
        };
        for (name, binding) in switches {
            let (Some(index), Some(slot)) = (
                binding.get("@fs_index").and_then(Json::as_i64),
                slot_of(dsp, name),
            ) else {
                unplaceable += 1;
                continue;
            };
            let primary = binding.get("@fs_primary").and_then(Json::as_bool) == Some(true);
            bindings.push((
                index as usize,
                primary,
                footswitch_entry(binding, slot, None),
            ));
        }
    }

    let Some(layout) = map_get_mut(&mut donor.preset, 3) else {
        return;
    };
    let Some(Value::Array(switches)) = map_get_mut(layout, 8) else {
        return;
    };
    let width = switches.len();
    switches.fill(Value::Nil);

    let mut off_device = 0usize;
    let mut written = 0usize;
    for switch in 1..=width {
        // Primary first — it is the one the pedal's screen names, and `10` records the order.
        let mut on_switch: Vec<&(usize, bool, Value)> =
            bindings.iter().filter(|(i, ..)| *i == switch).collect();
        on_switch.sort_by_key(|(_, primary, _)| !*primary);
        if on_switch.is_empty() {
            continue;
        }
        switches[switch - 1] = Value::Array(
            on_switch
                .iter()
                .enumerate()
                .map(|(position, (_, _, entry))| {
                    let mut entry = entry.clone();
                    set_map_key(&mut entry, 10, Value::from(position as i64));
                    entry
                })
                .collect(),
        );
        written += on_switch.len();
    }
    off_device += bindings
        .iter()
        .filter(|(i, ..)| *i > width || *i == 0)
        .count();

    out.footswitches = written;
    if off_device > 0 {
        out.not_carried.push(format!(
            "{off_device} footswitch binding(s): the preset puts them on switches the target \
             device does not have (it has {width})"
        ));
    }
    if unplaceable > 0 {
        out.not_carried.push(format!(
            "{unplaceable} footswitch binding(s): on something other than a block, so there is no \
             slot to point them at"
        ));
    }
}

/// One binding, in the device's own key order.
///
/// `15` and `16` are `false`/`0` on every entry of every fixture and nothing here varies them.
fn footswitch_entry(binding: &Json, slot: usize, param: Option<i64>) -> Value {
    let text = |key: &str| {
        binding
            .get(key)
            .and_then(Json::as_str)
            .map(|s| format!("{s}\0"))
    };
    let custom = text("@fs_customlabel");
    // A bypass binding is node type 1; a parameter controller on a switch is type 2 and carries
    // the parameter reference (`9`, same `{28, 29, 41}` shape as key 4's) plus a `2: 0`, in the
    // device's own key order. [solid — the oracle preset's "Route To" switch]
    let mut node = vec![
        (
            Value::from(0),
            Value::from(if param.is_some() { 2 } else { 1 }),
        ),
        (
            Value::from(5),
            Value::from(text("@fs_label").unwrap_or_else(|| "\0".into())),
        ),
        (
            Value::from(6),
            Value::from(
                binding
                    .get("@fs_ledcolor")
                    .and_then(Json::as_i64)
                    .unwrap_or(0),
            ),
        ),
        (
            Value::from(7),
            Value::from(binding.get("@fs_enabled").and_then(Json::as_bool) != Some(false)),
        ),
        (Value::from(8), Value::from(slot as i64)),
    ];
    if let Some(param_index) = param {
        node.push((Value::from(2), Value::from(0)));
        node.push((
            Value::from(9),
            Value::Map(vec![
                (Value::from(28), Value::from(0)),
                (Value::from(29), Value::from(param_index)),
                (Value::from(41), Value::from(false)),
            ]),
        ));
    }
    Value::Map(vec![
        (Value::from(10), Value::from(0)), // position on the switch, set by the caller
        (Value::from(11), Value::Map(node)),
        (
            Value::from(12),
            Value::from(binding.get("@fs_momentary").and_then(Json::as_bool) == Some(true)),
        ),
        (
            Value::from(14),
            Value::from(custom.clone().unwrap_or_else(|| "\0".into())),
        ),
        (Value::from(13), Value::from(custom.is_some())),
        (Value::from(16), Value::from(0)),
        (Value::from(15), Value::from(false)),
    ])
}

/// Empty the controller-assignment table (key `4`).
///
/// It addresses blocks by **slot number** and by parameter index, and the conversion has just
/// replaced every block in every slot. Leaving the donor's would not be a harmless omission like
/// the undecoded preset settings: each row would go on sweeping a parameter of whatever now sits
/// in that slot. Empty is a real device state — key `4` is entirely nil on a preset whose only
/// assignment is a bypass [solid — the `assign_bypass_on_fs1` fixture] — and a stale row is not.
///
/// Writing it from the `tone` is the remaining piece: the rows have to be built before the
/// snapshots' key `2`, which indexes off this table's order, can be written either.
/// One assignment written into preset key `4`, remembered by its tone address so the snapshots'
/// per-controller values (snapshot key `2`) can be written against the same places.
struct PlacedController {
    place: usize,
    dsp: usize,
    target: String,
    param: String,
}

/// Preset key `4` — the parameter-controller assignment table, written from the tone's
/// `controller` section.
///
/// The donor's own rows are cleared first whatever happens next: they address blocks by slot and
/// every slot has just been rewritten, so a kept row is a wrong one.
///
/// Row shape per `docs/preset-format.md` (`{0: source, 1: 4, 2: min, 3: max, 5: slot,
/// 6: {28: model path, 29: param index, 41}, 13: snapshot-disable}`), checked byte-for-byte
/// against all four rows of the one preset held in both forms — two expression pedals, one
/// footswitch, one snapshots-sourced. **Key `1` is written as the constant 4**: its semantics are
/// still an open question (see the docs — held samples disagree between 0 and 4), and 4 is what
/// every row of the oracle stores.
///
/// A **footswitch-sourced** assignment is also a **type-2 row in the footswitch layout**
/// (`3 → 8`) — the two tables describe one binding and must agree — so this returns those rows
/// for [`apply_footswitches`] to place alongside the bypass bindings. A **MIDI-sourced** one
/// additionally carries a CC number nothing in our `tone` samples shows, so it is skipped and
/// counted rather than half-written.
///
/// Places are handed out ascending (ordinal, encounter order), which reproduces the oracle's own
/// numbering; snapshot key `2` indexes by place, so any self-consistent order is functional.
fn apply_controllers(
    donor: &mut PresetStream,
    tone: &JsonMap<String, Json>,
    syms: &DeviceSymbols,
    out: &mut Conversion,
) -> (Vec<PlacedController>, Vec<(usize, bool, Value)>) {
    let table_len = match map_get_mut(&mut donor.preset, 4) {
        Some(Value::Array(rows)) => {
            if rows.iter().any(|c| !matches!(c, Value::Nil)) {
                out.not_carried.push(
                    "the target preset's own controller assignments: cleared, because they \
                     address blocks by slot and every slot has changed"
                        .into(),
                );
            }
            rows.fill(Value::Nil);
            rows.len()
        }
        _ => return (Vec::new(), Vec::new()),
    };
    // Source ordinals are sized by the device: 0 none, the expression inputs, one per footswitch,
    // then MIDI second-last and snapshots last (docs/preset-format.md). Where the footswitch run
    // starts is a device attribute: ordinal = switch number + 2 on the Stomp and the XL [solid,
    // both measured], + 5 on the Floor's 20-entry table [solid for the one pair held: the oracle's
    // "Route To" is ordinal 13 and sits at layout position 7 — plausibly EXP3 and the two Variax
    // knobs occupying 3..=5, which only a Floor has, but that reading is unconfirmed].
    let snapshots_ordinal = table_len as i64 - 1;
    let midi_ordinal = table_len as i64 - 2;
    let switch_offset = if table_len == 20 { 5 } else { 2 };

    let mut collected: Vec<(i64, PlacedController, Value, Option<(usize, bool)>)> = Vec::new();
    let mut midi_skipped = 0usize;
    for dsp in 0..DSP_GROUP_KEYS.len() {
        let dsp_key = format!("dsp{dsp}");
        let Some(ctl) = tone
            .get("controller")
            .and_then(|c| c.get(&dsp_key))
            .and_then(Json::as_object)
        else {
            continue;
        };
        let Some(tone_dsp) = tone.get(&dsp_key).and_then(Json::as_object) else {
            continue;
        };
        for (target, params) in ctl {
            let Some(params) = params.as_object() else {
                continue;
            };
            for (param, spec) in params {
                let Some(ordinal) = spec.get("@controller").and_then(Json::as_i64) else {
                    continue;
                };
                if ordinal == midi_ordinal {
                    midi_skipped += 1;
                    continue;
                }
                if ordinal <= 0 || ordinal > snapshots_ordinal {
                    out.not_carried.push(format!(
                        "controller dsp{dsp}.{target}.{param}: source ordinal {ordinal} is \
                         outside the target device's table — skipped"
                    ));
                    continue;
                }
                let on_switch = ordinal > switch_offset && ordinal < midi_ordinal;
                match controller_row(target, param, spec, ordinal, dsp, tone_dsp, syms) {
                    Ok(row) => {
                        // The layout half of a switch-sourced binding, at the switch the ordinal
                        // names. The slot is the row's own (key 5).
                        let switch = on_switch.then(|| {
                            let primary =
                                spec.get("@fs_primary").and_then(Json::as_bool) == Some(true);
                            ((ordinal - switch_offset) as usize, primary)
                        });
                        collected.push((
                            ordinal,
                            PlacedController {
                                place: 0,
                                dsp,
                                target: target.clone(),
                                param: param.clone(),
                            },
                            row,
                            switch,
                        ));
                    }
                    Err(why) => out.not_carried.push(format!(
                        "controller dsp{dsp}.{target}.{param}: {why} — skipped"
                    )),
                }
            }
        }
    }
    collected.sort_by_key(|(ordinal, ..)| *ordinal);

    let mut switch_rows: Vec<(usize, bool, Value)> = Vec::new();
    if let Some(Value::Array(rows)) = map_get_mut(&mut donor.preset, 4) {
        for (place, (ordinal, pc, row, _)) in collected.iter_mut().enumerate() {
            pc.place = place;
            let item = Value::Map(vec![
                (Value::from(0), Value::from(place as i64)),
                (Value::from(1), row.clone()),
            ]);
            match &mut rows[*ordinal as usize] {
                Value::Array(list) => list.push(item),
                slot => *slot = Value::Array(vec![item]),
            }
        }
    }
    for (_, pc, row, switch) in &collected {
        let Some((switch, primary)) = switch else {
            continue;
        };
        // Rebuild the layout row from the same tone entry the key-4 row came from.
        let dsp_key = format!("dsp{}", pc.dsp);
        let (Some(spec), Some(slot), Some(param_index)) = (
            tone.get("controller")
                .and_then(|c| c.get(&dsp_key))
                .and_then(|d| d.get(&pc.target))
                .and_then(|t| t.get(&pc.param)),
            map_get(row, 5).and_then(Value::as_i64),
            map_get(row, 6)
                .and_then(|p| map_get(p, 29))
                .and_then(Value::as_i64),
        ) else {
            continue;
        };
        switch_rows.push((
            *switch,
            *primary,
            footswitch_entry(spec, slot as usize, Some(param_index)),
        ));
    }
    out.controllers = collected.len();
    if midi_skipped > 0 {
        out.not_carried.push(format!(
            "{midi_skipped} MIDI-sourced controller assignment(s): skipped — a MIDI row also \
             carries a CC number no tone we hold shows"
        ));
    }
    (
        collected.into_iter().map(|(_, pc, ..)| pc).collect(),
        switch_rows,
    )
}

/// The inner map of one key-4 row (its key `1`), or why it cannot be built.
fn controller_row(
    target: &str,
    param: &str,
    spec: &Json,
    ordinal: i64,
    dsp: usize,
    tone_dsp: &JsonMap<String, Json>,
    syms: &DeviceSymbols,
) -> std::result::Result<Value, String> {
    let index = match target {
        "split" => Node::SPLIT.index,
        "join" => Node::MIXER.index,
        "inputA" => 0,
        "outputA" => 9,
        t if t.starts_with("block") => tone_dsp
            .get(t)
            .and_then(slot_index)
            .ok_or("its target block has no usable @path/@position")?,
        _ => {
            return Err(
                "a target of this kind has never been seen in a device-written table".into(),
            );
        }
    };
    let slot = dsp * DSP_SLOT_STRIDE as usize + index;
    let entry = tone_dsp
        .get(target)
        .ok_or("its target is not in the tone")?;
    let model = entry
        .get("@model")
        .and_then(Json::as_str)
        .ok_or("its target has no @model")?;
    let (_, _, params) = resolve_symbol(model, entry, syms).map_err(|e| e.to_string())?;
    let want = fold_param_name(param);
    let param_index = params
        .iter()
        .position(|p| fold_param_name(p) == want)
        .ok_or_else(|| format!("{param:?} is not a parameter of {model}"))?;
    Ok(Value::Map(vec![
        (Value::from(0), Value::from(ordinal)),
        (Value::from(1), Value::from(4)),
        (
            Value::from(2),
            json_to_msgpack(spec.get("@min").unwrap_or(&Json::Null)),
        ),
        (
            Value::from(3),
            json_to_msgpack(spec.get("@max").unwrap_or(&Json::Null)),
        ),
        (Value::from(4), Value::from(0)),
        (Value::from(5), Value::from(slot as i64)),
        (
            Value::from(6),
            Value::Map(vec![
                (Value::from(28), Value::from(0)),
                (Value::from(29), Value::from(param_index as i64)),
                (Value::from(41), Value::from(false)),
            ]),
        ),
        (Value::from(7), Value::from(0)),
        (
            Value::from(13),
            json_to_msgpack(spec.get("@snapshot_disable").unwrap_or(&Json::Bool(false))),
        ),
    ]))
}

/// Preset key `10` — the snapshots.
///
/// A snapshot is mostly a **per-slot bypass matrix** (`3`), one `[_, enabled]` pair per wire slot
/// across the whole device, plus its name, tempo and appearance. The `tone` keys that matrix by
/// block *name* (`blocks.dsp0.block3`), so it has to be re-indexed through the same
/// `@path`/`@position` → slot mapping the blocks themselves use — which is the reason this runs
/// after them and not beside them.
///
/// | tone | snapshot key | evidence |
/// |---|---:|---|
/// | `@valid` | `0` | |
/// | `blocks.dspN.<name>` | `3[wire slot][1]` | all 8 snapshots of the oracle preset |
/// | `@name` | `4`, NUL-terminated | `"Intro"` → `"Intro\0"` |
/// | `@tempo` | `5` | |
/// | `@pedalstate` | `11` | |
/// | `@ledcolor` | `12` | |
/// | `@custom_name` | `14` | |
///
/// Key `2` — the per-assignment controller values — is written against the places
/// [`apply_controllers`] handed out: one `[fs-enabled, place, value]` row per assignment, from
/// `snapshotN.controllers.dspN.<target>.<param>`, the rest of the array filled with the
/// `[false, len, nil]` sentinel the device uses for unused rows.
///
/// Left as the donor's: `1`, controller state of a shape not yet decoded (`[24, false, [0…]]` per
/// row in every fixture), and `10 → 8`, the stored active snapshot — the `tone` says 0 where the
/// same unit's dump says 4, so the pairing is unconfirmed and the field is already known to be an
/// unreliable reading of the live snapshot.
fn apply_snapshots(
    donor: &mut PresetStream,
    tone: &JsonMap<String, Json>,
    placed: &[PlacedController],
    out: &mut Conversion,
) {
    let placement = block_placement(tone);
    let Some(group) = map_get_mut(&mut donor.preset, 10) else {
        return;
    };
    let Some(Value::Array(list)) = map_get_mut(group, 10) else {
        return;
    };
    let on_device = list.len();
    let in_tone = (0..)
        .take_while(|i| tone.contains_key(&format!("snapshot{i}")))
        .count();
    if in_tone > on_device {
        out.not_carried.push(format!(
            "{} of the preset's {in_tone} snapshots: the target device holds {on_device}",
            in_tone - on_device
        ));
    }

    for (i, snapshot) in list.iter_mut().enumerate().take(in_tone) {
        let Some(from) = tone.get(&format!("snapshot{i}")) else {
            continue;
        };
        for (tone_key, wire_key) in [
            ("@valid", 0i64),
            ("@tempo", 5),
            ("@pedalstate", 11),
            ("@ledcolor", 12),
            ("@custom_name", 14),
        ] {
            if let Some(v) = from.get(tone_key) {
                set_map_key(snapshot, wire_key, json_to_msgpack(v));
            }
        }
        if let Some(name) = from.get("@name").and_then(Json::as_str) {
            set_map_key(snapshot, 4, Value::from(format!("{name}\0")));
        }
        // The matrix keeps whatever length the target device uses — 20 slots per DSP.
        let width = match map_get(snapshot, 3) {
            Some(Value::Array(a)) => a.len(),
            _ => continue,
        };
        set_map_key(snapshot, 3, bypass_matrix(from, &placement, width));

        // Key 2 — one row per assignment place; sentinel [false, len, nil] where nothing is.
        let len2 = match map_get(snapshot, 2) {
            Some(Value::Array(a)) => a.len(),
            _ => 0,
        };
        if len2 > 0 {
            let mut rows: Vec<Value> = (0..len2)
                .map(|_| {
                    Value::Array(vec![
                        Value::from(false),
                        Value::from(len2 as i64),
                        Value::Nil,
                    ])
                })
                .collect();
            for pc in placed.iter().filter(|p| p.place < len2) {
                let spec = from
                    .get("controllers")
                    .and_then(|c| c.get(format!("dsp{}", pc.dsp)))
                    .and_then(|d| d.get(&pc.target))
                    .and_then(|t| t.get(&pc.param));
                let fs_enabled = spec
                    .and_then(|s| s.get("@fs_enabled"))
                    .and_then(Json::as_bool)
                    .unwrap_or(false);
                let value = spec.and_then(|s| s.get("@value"));
                rows[pc.place] = Value::Array(vec![
                    Value::from(fs_enabled),
                    Value::from(pc.place as i64),
                    value.map(json_to_msgpack).unwrap_or(Value::Nil),
                ]);
            }
            set_map_key(snapshot, 2, Value::Array(rows));
        }
    }
    out.snapshots = in_tone.min(on_device);
}

/// Every `tone` block's wire slot, as `(dsp, tone name) → wire slot`.
///
/// The structural nodes are in here too: a snapshot can bypass the split, and the `tone` names it
/// `split` alongside the blocks.
fn block_placement(tone: &JsonMap<String, Json>) -> Vec<((usize, String), usize)> {
    let mut out = Vec::new();
    for dsp in 0..DSP_GROUP_KEYS.len() {
        let Some(tone_dsp) = tone.get(&format!("dsp{dsp}")).and_then(Json::as_object) else {
            continue;
        };
        for (name, block) in tone_dsp {
            let index = match name.as_str() {
                "split" => Some(Node::SPLIT.index),
                "join" => Some(Node::MIXER.index),
                "inputA" => Some(0),
                "outputA" => Some(9),
                n if n.starts_with("block") => slot_index(block),
                _ => None,
            };
            if let Some(index) = index {
                out.push(((dsp, name.clone()), dsp * DSP_SLOT_STRIDE as usize + index));
            }
        }
    }
    out
}

/// One snapshot's per-slot bypass matrix: `[_, enabled]` per wire slot, across every DSP.
///
/// Slots the `tone` says nothing about default to **enabled**, which is what the device writes for
/// an empty slot — except the input, output and mixer slots of each DSP, which are always `false`.
/// The first element of each pair is `false` throughout on every snapshot of every fixture we hold
/// and discriminates nothing.
fn bypass_matrix(snapshot: &Json, placement: &[((usize, String), usize)], width: usize) -> Value {
    let stride = DSP_SLOT_STRIDE as usize;
    let mut enabled: Vec<bool> = (0..width)
        .map(|slot| !matches!(slot % stride, 0 | 9 | 19))
        .collect();
    for ((dsp, name), slot) in placement {
        let Some(state) = snapshot
            .get("blocks")
            .and_then(|b| b.get(format!("dsp{dsp}")))
            .and_then(|d| d.get(name))
            .and_then(Json::as_bool)
        else {
            continue;
        };
        if let Some(cell) = enabled.get_mut(*slot) {
            *cell = state;
        }
    }
    Value::Array(
        enabled
            .into_iter()
            .map(|e| Value::Array(vec![Value::from(false), Value::from(e)]))
            .collect(),
    )
}

/// Empty every draggable slot of one DSP, so a donor block never survives into the result.
fn clear_blocks(donor: &mut PresetStream, group_key: i64) {
    let Some(group) = map_get_mut(&mut donor.preset, group_key) else {
        return;
    };
    let Some(Value::Array(slots)) = map_get_mut(group, 22) else {
        return;
    };
    for (i, slot) in slots.iter_mut().enumerate() {
        let is_block_slot = matches!(i % ROW_STRIDE, 1..=ROW_WIDTH) && i < 2 * ROW_STRIDE;
        if !is_block_slot {
            continue; // input, output, split and mixer nodes are not ours to clear
        }
        set_map_key(slot, 19, Value::from(slot_kind::EMPTY));
        set_map_key(slot, 20, Value::Nil);
    }
}

/// `@path × 10 + @position + 1` — a `tone` block's slot in its DSP's 20-slot array.
fn slot_index(block: &Json) -> Option<usize> {
    let path = block.get("@path")?.as_i64()? as usize;
    let position = block.get("@position")?.as_i64()? as usize;
    (path < 2 && position < ROW_WIDTH).then_some(path * ROW_STRIDE + position + 1)
}

/// Build one `type 6` effect slot from a `tone` block.
///
/// `siblings` is the block's `dspN` object, which is where an `@cab` reference resolves — the
/// paired cab of an amp+cab (`@type` 3) or the second cab of a dual (`@type` 4) is a sibling
/// entry of the same map, not a nested object.
fn encode_block(
    block: &Json,
    siblings: &JsonMap<String, Json>,
    syms: &DeviceSymbols,
) -> Result<Value> {
    let model = block
        .get("@model")
        .and_then(Json::as_str)
        .ok_or_else(|| Error::Stream("no @model".into()))?;
    let tone_type = block.get("@type").and_then(Json::as_i64).unwrap_or(0);
    if tone_type == 6 {
        return encode_looper(block, model, syms);
    }

    let (index, symbol, params) = resolve_symbol(model, block, syms)?;
    let (values, symbol_count) = param_values(block, params);

    // The paired model, where the type carries one. `@cab` on a type this has never been seen on
    // stays a refusal: the pairing classes were measured per type, not assumed transferable.
    let cab_ref = block.get("@cab").and_then(Json::as_str);
    let paired = match (tone_type, cab_ref) {
        (3 | 4, Some(name)) => {
            let sibling = siblings.get(name).ok_or_else(|| {
                Error::Stream(format!(
                    "{model} names a cab sibling {name:?} that is not in the tone"
                ))
            })?;
            let sib_model = sibling
                .get("@model")
                .and_then(Json::as_str)
                .ok_or_else(|| Error::Stream(format!("cab sibling {name:?} has no @model")))?;
            let (sib_index, sib_symbol, sib_params) = resolve_symbol(sib_model, sibling, syms)?;
            // Both live measurements paired like with like, and the backup holds no mixed pair
            // (0 of 78 duals), so a mixed one is a shape the device has never shown us.
            if tone_type == 4
                && sib_symbol.starts_with("HD2_CabMicIr") != symbol.starts_with("HD2_CabMicIr")
            {
                return Err(Error::Stream(format!(
                    "{model} pairs a legacy and a new-family cab ({sib_model}) — no device-written \
                     preset we hold mixes the families in one block"
                )));
            }
            let (sib_values, sib_symbol_count) = param_values(sibling, sib_params);
            Some((sib_index, sib_symbol, sib_values, sib_symbol_count, sibling))
        }
        (_, Some(name)) => {
            return Err(Error::Stream(format!(
                "{model} is a @type {tone_type} block carrying a cab sibling {name:?} — a shape no \
                 device-written preset we hold contains"
            )));
        }
        (_, None) => None,
    };

    let class = block_class(
        tone_type,
        &symbol,
        paired.as_ref().map(|(_, s, ..)| s.as_str()),
    )
    .ok_or_else(|| {
        Error::Stream(format!(
            "{model} is a @type {tone_type} block and no device-written preset we hold contains \
                 one, so its block class is unknown — converting it would mean guessing"
        ))
    })?;

    // `24 → 23` is the paired-model-active flag; the sibling's own `@enabled` is the only tone
    // field it can correspond to, and no backup preset has ever carried it false. [hypothesis for
    // the false case — every observed pair is (present, true)]
    let (paired_index, paired_active, paired_bank) = match paired {
        Some((idx, _, vals, sym_count, sibling)) => (
            idx as i64,
            sibling
                .get("@enabled")
                .and_then(Json::as_bool)
                .unwrap_or(true),
            param_bank(vals.len() as i64, sym_count, vals),
        ),
        // The second bank is empty on every unpaired block in every fixture.
        None => (-1, false, param_bank(0, 0, Vec::new())),
    };

    let mut content = vec![
        (
            Value::from(24),
            Value::Map(vec![
                (Value::from(23), Value::from(paired_active)),
                (Value::from(25), Value::from(index as i64)),
                (Value::from(26), Value::from(paired_index)),
            ]),
        ),
        (Value::from(9), Value::from(class)),
        (
            Value::from(10),
            Value::from(
                block
                    .get("@enabled")
                    .and_then(Json::as_bool)
                    .unwrap_or(true),
            ),
        ),
        (
            Value::from(11),
            param_bank(values.len() as i64, symbol_count, values),
        ),
    ];
    if tone_type == 5 {
        // An IR block has no second bank at all; in its place the content carries key 27, the
        // referenced IR's UUID, NUL-terminated — the tone's `@uuid`, and how the device re-matches
        // the IR by content when slots have moved. A dual IR concatenates its two UUIDs into the
        // one string (64 hex chars, measured). A block aimed at an **empty** IR slot stores the
        // empty string, and a tone with no `@uuid` is that case.
        // [solid — class_ir_{mono,dual,unreferenced} fixtures, 2026-08-26]
        let uuid = block.get("@uuid").and_then(Json::as_str).unwrap_or("");
        content.push((Value::from(27), Value::from(format!("{uuid}\0"))));
    } else {
        content.push((Value::from(12), paired_bank));
    }
    Ok(Value::Map(vec![
        (Value::from(19), Value::from(slot_kind::EFFECT)),
        (Value::from(20), Value::Map(content)),
    ]))
}

/// Build a **Looper** slot (`@type` 6) — a different slot kind (7, not 6) with its own content
/// shape: model index at key `8`, class at `9` (**22**), enabled at `10`, params at `7 → 4`, in
/// that key order, with no model-ref map and no second bank.
///
/// The device stores only the looper's *preset* parameters — `Playback`, `Overdub`, `lowCut`,
/// `highCut`, 4 of the symbol's 10; the rest (`Reverse`, `Undo`, `PendingState`, …) are live
/// transport state. The tone carries exactly the stored set, so the vector is the symbol-ordered
/// params the tone actually has, not a nil-padded full list.
///
/// [solid — the "Sultans" Floor stream holds a device-written looper slot and the same unit's
/// `.hxb` holds its tone; `tests/paired_blocks.rs` compares the two]
fn encode_looper(block: &Json, model: &str, syms: &DeviceSymbols) -> Result<Value> {
    let (index, _, params) = resolve_symbol(model, block, syms)?;
    let values: Vec<Value> = params
        .iter()
        .filter_map(|name| get_param(block, name))
        .map(json_to_msgpack)
        .collect();
    let count = values.len() as i64;
    let content = Value::Map(vec![
        (Value::from(8), Value::from(index as i64)),
        (Value::from(9), Value::from(22)),
        (
            Value::from(10),
            Value::from(
                block
                    .get("@enabled")
                    .and_then(Json::as_bool)
                    .unwrap_or(true),
            ),
        ),
        (Value::from(7), param_bank(count, count, values)),
    ]);
    Ok(Value::Map(vec![
        (Value::from(19), Value::from(slot_kind::LOOPER)),
        (Value::from(20), content),
    ]))
}

/// A block's stored value vector, ordered by its symbol's parameter list, plus the count the wire
/// puts beside it at key `3`.
///
/// Two measured rules shape it ([solid — HX Stomp sweep, 2026-08-26]):
/// - **`IrData` is never stored.** A new-family cab's symbol ends with it and the device writes
///   the vector without it (7 of 8 on a plain cab, 9 of 10 on a `WithPan`), so key `3` counts the
///   storable parameters, not the symbol's.
/// - **Extras append past the symbol's list.** A legacy cab's `@mic` and a delay/reverb's
///   `@trails` land after the last symbol parameter and are counted by key `2` but not key `3`.
///
/// Parameter names are matched with spaces stripped and case folded — the tone spells the same
/// parameter `HighCut` or `High Cut` depending on which HX Edit era wrote it.
fn param_values(block: &Json, params: &[String]) -> (Vec<Value>, i64) {
    let mut values: Vec<Value> = params
        .iter()
        .filter(|name| *name != "IrData")
        .map(|name| json_to_msgpack(get_param(block, name).unwrap_or(&Json::Null)))
        .collect();
    let symbol_count = values.len() as i64;
    if let Some(mic) = block.get("@mic") {
        values.push(json_to_msgpack(mic));
    }
    if let Some(trails) = block.get("@trails") {
        values.push(json_to_msgpack(trails));
    }
    (values, symbol_count)
}

/// Look a parameter up by its device-symbol name, tolerating the tone's spelling drift.
///
/// One real backup stores the same legacy cab's high cut as `HighCut` 142 times and `High Cut` 26,
/// with a stray `Low Cut` and `Early Reflections` besides — the spelling tracks the HX Edit era
/// that wrote the preset, while `Helix.sym` spells it without the space. Exact match first, then
/// space-stripped case-folded.
fn get_param<'a>(block: &'a Json, name: &str) -> Option<&'a Json> {
    if let Some(v) = block.get(name) {
        return Some(v);
    }
    let want = fold_param_name(name);
    block
        .as_object()?
        .iter()
        .find(|(k, _)| !k.starts_with('@') && fold_param_name(k) == want)
        .map(|(_, v)| v)
}

fn fold_param_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// `{2: stored, 3: from the symbol, 4: [values]}` — the shape both effect blocks and structural
/// nodes use for a parameter vector.
fn param_bank(stored: i64, from_symbol: i64, values: Vec<Value>) -> Value {
    Value::Map(vec![
        (Value::from(2), Value::from(stored)),
        (Value::from(3), Value::from(from_symbol)),
        (Value::from(4), Value::Array(values)),
    ])
}

/// A `tone` model name + its `@stereo` flag → the device symbol's index and parameter order.
///
/// The device symbol carries a `Mono`/`Stereo` suffix and the two variants have **different**
/// parameter orders and counts, which is the whole reason [`crate::symbols`] exists. Picking the
/// wrong one silently misaligns every value past the first variant-only parameter.
///
/// **`@stereo` is only written when there is a choice to make.** Of the 680 models in `Helix.sym`,
/// 153 have both variants and carry the flag; the rest have one form or none — reverbs are
/// `Stereo`-only, IRs `Mono`-only, amps and cabs unsuffixed — and HX Edit simply omits it. So an
/// absent flag is not "assume Mono", it is "take the one that exists", and treating it as the
/// former is what made `HD2_ReverbHall` unresolvable.
fn resolve_symbol<'a>(
    model: &str,
    block: &Json,
    syms: &'a DeviceSymbols,
) -> Result<(usize, String, &'a [String])> {
    let asked = match block.get("@stereo").and_then(Json::as_bool) {
        Some(true) => Some("Stereo"),
        Some(false) => Some("Mono"),
        None => None,
    };
    let available = syms.variants_of(model);
    let suffix = match (asked, available.as_slice()) {
        // The flag names a variant the device has: use it.
        (Some(asked), avail) if avail.contains(&asked) => asked,
        // No choice to make — the flag is absent, or names a variant this model doesn't have.
        (_, [only]) => only,
        (_, []) => {
            return Err(Error::Stream(format!(
                "{model:?} is not in Helix.sym — the reference data is from a different HX Edit \
                 version than the preset"
            )));
        }
        // Both variants exist and the tone didn't say which. Guessing here misaligns the whole
        // value vector, so refuse and say so.
        (None, _) => {
            return Err(Error::Stream(format!(
                "{model:?} has both a Mono and a Stereo form and the preset carries no @stereo \
                 flag, so which one it means cannot be determined"
            )));
        }
        (Some(_), avail) => avail[0],
    };
    let symbol = format!("{model}{suffix}");
    let index = syms
        .index_of(&symbol)
        .ok_or_else(|| Error::Stream(format!("{symbol:?} is not in Helix.sym")))?;
    let params = syms.by_index(index).map(|(_, p)| p).unwrap_or(&[]);
    Ok((index, symbol, params))
}

/// Write a built slot into a DSP's slot array.
fn write_slot(donor: &mut PresetStream, group_key: i64, index: usize, slot: Value) -> Result<()> {
    let group = map_get_mut(&mut donor.preset, group_key)
        .ok_or_else(|| Error::Stream(format!("preset key {group_key} missing")))?;
    let Some(Value::Array(slots)) = map_get_mut(group, 22) else {
        return Err(Error::Stream(format!(
            "preset key {group_key} has no slot array"
        )));
    };
    let target = slots
        .get_mut(index)
        .ok_or_else(|| Error::Stream(format!("slot {index} is past the end of the array")))?;
    *target = slot;
    Ok(())
}

/// Overlay one structural node (`split` or `join`) — its model, its parameters, whether it is a
/// real branch point on this DSP, and where along the row it sits.
///
/// The node slots are always present, on serial presets too, so this only ever updates them.
///
/// **Whether a node is active comes from the topology string, not from its own `@enabled`.**
/// Both nodes of "Pull Me Under" say `@enabled: true` on both DSPs, and the device has DSP1's join
/// inactive and DSP2's split inactive — because that preset's bracket *opens* on DSP1 and *closes*
/// on DSP2 (`SAB` then `ABJ`). Reading `@enabled` puts a join point on the DSP where the paths are
/// still running, which is a preset whose two signal paths recombine a whole DSP too early.
/// `@enabled` is the node's own bypass and lands on the holder's key `10`. [solid — 2026-08-25]
///
/// An inactive node carries column 0 whatever its `@position` says, which is what the device writes.
fn apply_node(
    donor: &mut PresetStream,
    group_key: i64,
    tone_dsp: &JsonMap<String, Json>,
    node_kind: Node,
    topology: Option<&str>,
    syms: &DeviceSymbols,
    out: &mut Conversion,
) {
    let tone_key = node_kind.tone_key;
    let Some(node) = tone_dsp.get(tone_key) else {
        return;
    };
    let Some(model) = node.get("@model").and_then(Json::as_str) else {
        return;
    };
    let Ok((sym_index, _, params)) = resolve_symbol(model, node, syms) else {
        out.not_carried.push(format!(
            "{tone_key}: {model} is not in Helix.sym, left as the target's"
        ));
        return;
    };
    let Some(topology) = topology else {
        out.not_carried.push(format!(
            "{tone_key}: no topology string, so whether it is a branch point was left as the \
             target's"
        ));
        return;
    };
    let values: Vec<Value> = params
        .iter()
        .map(|p| json_to_msgpack(node.get(p.as_str()).unwrap_or(&Json::Null)))
        .collect();
    let count = values.len() as i64;
    let active = topology.contains(node_kind.marker);
    let column = match active {
        true => node.get("@position").and_then(Json::as_i64).unwrap_or(0) + 1,
        false => 0,
    };
    let enabled = node.get("@enabled").and_then(Json::as_bool).unwrap_or(true);

    let Some(group) = map_get_mut(&mut donor.preset, group_key) else {
        return;
    };
    let Some(Value::Array(slots)) = map_get_mut(group, 22) else {
        return;
    };
    let Some(slot) = slots.get_mut(node_kind.index) else {
        return;
    };
    let Some(content) = map_get_mut(slot, 20) else {
        return;
    };
    // The model holder is the sub-map carrying key 8 — 15 for a split, 17 for a mixer.
    let Value::Map(entries) = content else {
        return;
    };
    let Some(holder) = entries
        .iter_mut()
        .map(|(_, v)| v)
        .find(|v| map_get(v, 8).is_some())
    else {
        return;
    };
    set_map_key(holder, 8, Value::from(sym_index as i64));
    set_map_key(holder, 13, Value::from(column));
    set_map_key(holder, 10, Value::from(enabled));
    set_map_key(holder, 7, param_bank(count, count, values));
    // `18` sits beside the holder, not inside it: it is the node content's own flag.
    set_map_key(content, 18, Value::from(active));
}

/// This DSP's entry in `global.@topologyN`.
fn dsp_topology(tone: &JsonMap<String, Json>, dsp: usize) -> Option<&str> {
    tone.get("global")?.get(format!("@topology{dsp}"))?.as_str()
}

/// Set the DSP group's split type (key `21`) from `global.@topologyN`.
///
/// A topology string with no measured code leaves the donor's value and is reported, rather than
/// being mapped to the nearest plausible number — see [`topology_code`].
fn apply_topology(
    donor: &mut PresetStream,
    group_key: i64,
    topology: Option<&str>,
    dsp: usize,
    out: &mut Conversion,
) {
    let Some(topology) = topology else {
        return;
    };
    let Some(code) = topology_code(topology) else {
        out.not_carried.push(format!(
            "dsp{dsp}: topology {topology:?} has no measured split type, left as the target's"
        ));
        return;
    };
    if let Some(group) = map_get_mut(&mut donor.preset, group_key) {
        set_map_key(group, 21, Value::from(code));
    }
}

/// One line per part of the `tone` tree this does not write.
fn note_omissions(tone: &JsonMap<String, Json>, out: &mut Conversion) {
    if out.snapshots > 0 {
        out.not_carried.push(
            "snapshot key 1 (controller state of undecoded shape) and the stored active snapshot: \
             left as the target preset's"
                .into(),
        );
    }
    for (key, what) in [
        (
            "global",
            "preset tempo, input impedance/pad and the focused block",
        ),
        ("irUuidTable", "the IR slot references"),
        ("variax", "Variax settings"),
    ] {
        if tone.contains_key(key) {
            out.not_carried.push(format!("{key}: {what}"));
        }
    }
    out.not_carried
        .push("input and output nodes: gate, gain and routing left as the target preset's".into());
}

/// A `tone` parameter value in the encoding the wire uses.
///
/// The device stores knobs as `float32` and enums/switches as integers and bools, and the `tone`
/// JSON preserves that distinction: a knob is a JSON float, a switch a JSON bool, an enum a JSON
/// integer. Round-tripping the type matters — a value written as an int where the device expects a
/// float reads as a different number entirely.
fn json_to_msgpack(v: &Json) -> Value {
    match v {
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) if n.is_i64() && !n.to_string().contains('.') => {
            Value::from(n.as_i64().unwrap_or(0))
        }
        Json::Number(n) => Value::F32(n.as_f64().unwrap_or(0.0) as f32),
        Json::String(s) => Value::from(s.as_str()),
        // A parameter the tone doesn't carry. The device has never shown us a hole in a value
        // vector, so this is a shape we are inventing; f32 0.0 is the least surprising filler and
        // the caller sees the block it happened on in the diff.
        _ => Value::F32(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_index_is_row_times_ten_plus_column() {
        let at = |path, pos| slot_index(&serde_json::json!({ "@path": path, "@position": pos }));
        // Row A occupies 1..=8, row B 11..=18 — the layout `dsp_grid` draws.
        assert_eq!(at(0, 0), Some(1));
        assert_eq!(at(0, 7), Some(8));
        assert_eq!(at(1, 0), Some(11));
        assert_eq!(at(1, 7), Some(18));
        // Past the end of a row would land on the output or mixer node.
        assert_eq!(at(0, 8), None);
        assert_eq!(at(2, 0), None);
    }

    #[test]
    fn only_observed_block_classes_convert() {
        // Every class here was read off a device-written preset — the fixtures, or the 2026-08-26
        // live sweep on an HX Stomp (see the fn's own table).
        let legacy = "HD2_Cab1x12USDeluxe";
        let micir = "HD2_CabMicIr_1x12USDeluxe";
        assert_eq!(block_class(0, "HD2_Tremolo", None), Some(1));
        assert_eq!(block_class(1, "HD2_AmpUSDoubleNrm", None), Some(17));
        assert_eq!(block_class(2, legacy, None), Some(15));
        assert_eq!(block_class(2, micir, None), Some(31));
        assert_eq!(block_class(3, "HD2_AmpUSDoubleNrm", Some(legacy)), Some(18));
        assert_eq!(block_class(3, "HD2_AmpUSDoubleNrm", Some(micir)), Some(33));
        // An amp+cab with no cab sibling is a shape we have never seen — refused, not defaulted.
        assert_eq!(block_class(3, "HD2_AmpUSDoubleNrm", None), None);
        assert_eq!(block_class(4, legacy, Some(legacy)), Some(16));
        assert_eq!(block_class(4, micir, Some(micir)), Some(32));
        assert_eq!(
            block_class(5, "HD2_ImpulseResponse1024Mono", None),
            Some(19)
        );
        assert_eq!(
            block_class(5, "HD2_ImpulseResponse1024DualStereo", None),
            Some(21)
        );
        assert_eq!(block_class(7, "HD2_DelaySimpleDelay", None), Some(8));
        assert_eq!(
            block_class(8, "HD2_Synth3NoteGeneratorMono", None),
            Some(23)
        );
        // The looper is a different slot shape entirely (kind 7), never built as an effect slot.
        assert_eq!(block_class(6, "HD2_Looper", None), None, "@type 6");
    }

    #[test]
    fn param_lookup_survives_the_tones_spelling_drift() {
        // One real backup spells the same legacy cab's high cut `HighCut` 142 times and
        // `High Cut` 26 — the era of the HX Edit that wrote the preset, not the model.
        let block = serde_json::json!({ "High Cut": 8000.0, "LowCut": 80.0, "@mic": 6 });
        assert_eq!(
            get_param(&block, "HighCut"),
            Some(&serde_json::json!(8000.0))
        );
        assert_eq!(get_param(&block, "LowCut"), Some(&serde_json::json!(80.0)));
        // Meta keys never answer a parameter lookup, whatever they fold to.
        assert_eq!(get_param(&block, "mic"), None);
    }

    #[test]
    fn value_types_survive_the_crossing() {
        // A knob is a float, a switch a bool, an enum an int — and the device tells them apart.
        assert!(matches!(
            json_to_msgpack(&serde_json::json!(0.5)),
            Value::F32(_)
        ));
        assert!(matches!(
            json_to_msgpack(&serde_json::json!(true)),
            Value::Boolean(true)
        ));
        assert!(matches!(
            json_to_msgpack(&serde_json::json!(6)),
            Value::Integer(_)
        ));
        // A JSON float that happens to be whole is still a knob: `Headroom: 12.0` is f32 12, not
        // int 12, and the two are different bytes on the wire.
        assert!(matches!(
            json_to_msgpack(&serde_json::json!(12.0)),
            Value::F32(_)
        ));
    }
}
