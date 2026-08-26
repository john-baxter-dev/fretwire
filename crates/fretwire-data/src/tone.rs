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
//! | `@cab` | paired cab index (`24 → 26`), with `24 → 23` = true |
//! | `@mic`, `@trails` | one extra value appended past the symbol's parameters |
//! | `@type` | content key `9`, the block class |
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
//! That is the conservative direction for the same reason the rest of this crate takes it: a blob
//! is written to flash, and a field we guessed wrong is not visible until a musician's preset
//! sounds different. [`Conversion::not_carried`] names every such omission rather than leaving the
//! caller to assume the conversion was total.
//!
//! For the same reason a block whose class we have never seen on the wire is a **refusal**, not a
//! guess — see [`block_class`].

use serde_json::{Map as JsonMap, Value as Json};

use crate::stream::{DSP_GROUP_KEYS, PresetStream, map_get, map_get_mut, set_map_key, slot_kind};
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
    /// Parts of the `tone` tree this does not yet write, one human-readable line each.
    pub not_carried: Vec<String>,
}

/// A `tone` block's `@type` → the wire's block-class byte (content key `9`).
///
/// Every value here was read off a device-authored preset, paired with the `tone` block it
/// encodes. The four types with no row are **not** guessed: nothing we hold shows what the device
/// writes for them, and a wrong class on a preset bound for flash is not a mistake worth making to
/// save a capture. They need one wire dump each of a preset containing that kind of block.
///
/// | `@type` | class | evidence |
/// |---:|---:|---|
/// | 0 | 1 | every non-amp, non-cab, non-delay block in all six fixtures |
/// | 1 | 17 | `AmpJazzRivet120`, `AmpCaliRectifire` — amp with no cab, `24 → 26` = −1 |
/// | 2 | 15 | `Cab2x12JazzRivet`, `Cab4X12CaliV30` |
/// | 3 | 33 | `AmpUSPrincess`, `AmpGSG100`, `AmpLine6Litigator` — each paired, `24 → 23` true |
/// | 7 | 8 | every delay and reverb, and exactly the blocks carrying a trailing `@trails` |
///
/// [solid — 2026-08-25; the 17/33 split is pinned from both sides, in that `class == 33` and
/// `24 → 26 != -1` and `24 → 23 == true` coincide on every amp in every fixture we hold]
pub fn block_class(tone_type: i64) -> Option<i64> {
    match tone_type {
        0 => Some(1),
        1 => Some(17),
        2 => Some(15),
        3 => Some(33),
        7 => Some(8),
        // 4 (cab + cab), 5 (IR), 6 (looper — a different slot shape entirely, kind 7 not 6) and
        // 8 (synth) all appear in the backup and in none of our wire dumps.
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
            let encoded = encode_block(block, syms)
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

    note_omissions(tone, &mut out);
    Ok(out)
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
fn encode_block(block: &Json, syms: &DeviceSymbols) -> Result<Value> {
    let model = block
        .get("@model")
        .and_then(Json::as_str)
        .ok_or_else(|| Error::Stream("no @model".into()))?;
    let tone_type = block.get("@type").and_then(Json::as_i64).unwrap_or(0);
    let class = block_class(tone_type).ok_or_else(|| {
        Error::Stream(format!(
            "{model} is a @type {tone_type} block and no device-written preset we hold contains \
             one, so its block class is unknown — converting it would mean guessing"
        ))
    })?;

    let (index, params) = resolve_symbol(model, block, syms)?;
    let mut values: Vec<Value> = Vec::with_capacity(params.len() + 1);
    for name in params {
        values.push(json_to_msgpack(
            block.get(name.as_str()).unwrap_or(&Json::Null),
        ));
    }
    let symbol_count = values.len() as i64;

    // The one or two values stored past the symbol's parameter list. A cab appends its mic, and
    // anything trails-capable appends the switch; no block we hold carries both.
    if let Some(mic) = block.get("@mic") {
        values.push(json_to_msgpack(mic));
    }
    if let Some(trails) = block.get("@trails") {
        values.push(json_to_msgpack(trails));
    }

    let paired = match block.get("@cab").and_then(Json::as_str) {
        Some(cab) => syms
            .index_of(cab)
            .ok_or_else(|| Error::Stream(format!("paired cab {cab:?} is not in Helix.sym")))?
            as i64,
        None => -1,
    };

    let content = Value::Map(vec![
        (
            Value::from(24),
            Value::Map(vec![
                (Value::from(23), Value::from(paired >= 0)),
                (Value::from(25), Value::from(index as i64)),
                (Value::from(26), Value::from(paired)),
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
        // The second bank is empty on every block in every fixture.
        (Value::from(12), param_bank(0, 0, Vec::new())),
    ]);
    Ok(Value::Map(vec![
        (Value::from(19), Value::from(slot_kind::EFFECT)),
        (Value::from(20), content),
    ]))
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
) -> Result<(usize, &'a [String])> {
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
    Ok((index, syms.by_index(index).map(|(_, p)| p).unwrap_or(&[])))
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
    let Ok((sym_index, params)) = resolve_symbol(model, node, syms) else {
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
    let snapshots = (0..8)
        .filter(|i| tone.contains_key(&format!("snapshot{i}")))
        .count();
    if snapshots > 0 {
        out.not_carried.push(format!(
            "{snapshots} snapshot(s): names, per-block bypass and controller values were left as \
             the target preset's"
        ));
    }
    for (key, what) in [
        (
            "footswitch",
            "footswitch bindings, custom labels and LED colours",
        ),
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
        // Every class here was read off a device-written preset.
        assert_eq!(block_class(0), Some(1));
        assert_eq!(block_class(1), Some(17));
        assert_eq!(block_class(2), Some(15));
        assert_eq!(block_class(3), Some(33));
        assert_eq!(block_class(7), Some(8));
        // These appear in a real backup and in no wire dump we hold. They must stay refusals until
        // one turns up — a guessed class goes to flash and shows up as a preset that sounds wrong.
        for unobserved in [4, 5, 6, 8] {
            assert_eq!(block_class(unobserved), None, "@type {unobserved}");
        }
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
