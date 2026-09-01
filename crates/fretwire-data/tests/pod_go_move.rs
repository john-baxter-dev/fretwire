//! POD Go **move** against its strongest possible oracle: the same preset captured off the same
//! pedal minutes apart, before and after its owner dragged the volume block from slot 1 to slot
//! 10 in POD Go Edit (issue #15, 2026-08-28).
//!
//! POD Go Edit realizes a move as a whole-document rewrite (op 78 on the source slot, then the
//! rewritten preset as an op 21 write — the editor's only structural verb on this device), so
//! `PresetStream::move_block_single_row` must reproduce that rewrite. The "before" is the second-
//! IR-preset startup capture's stream (device-written); the "after" is the op-21 blob lifted out
//! of the move capture (POD Go Edit-written). Our move rearranges the device's own document, so
//! the comparison runs after normalizing the ways POD Go Edit's serializer re-spells content the
//! pedal round-trips back to the device form — each one asserted exactly before it is erased,
//! so a change in either source fails loudly:
//!
//! * a footswitch binding's key `11 → 2` is dropped when it is `0` (the pre stream spells the
//!   zero out, the rewrite omits it);
//! * a controller row's assignments come back sorted by their key-`0` id (the pre stream holds
//!   them in insertion order);
//! * the IR block is written with its **five symbol parameters only** and a **bare uuid** where
//!   the device's read-back carries a sixth, device-generated value and a NUL terminator — the
//!   proof that the sixth value is derived device state, not preset content (it never leaves
//!   the editor), which is what lifted `hxb-convert`'s IR refusal;
//! * the EQ goes out as class `1` and the FX loop as class `8` (the serializer's own
//!   vocabulary) where the device re-emits `23` and `9` — the pedal accepts both spellings and
//!   normalizes on read-back.
//!
//! Personal device data: both files stay out of git, so everything here skips on a clean clone.

use fretwire_data::stream::{PresetStream, map_get};
use rmpv::Value;
use std::path::PathBuf;

fn captures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/pod-go")
}

fn before() -> Option<PresetStream> {
    PresetStream::parse(&std::fs::read(captures().join("ir2-preset.msgpack.bin")).ok()?).ok()
}

/// The op-21 blob from the move capture, wrapped in the read-reply envelope `parse` expects.
fn after() -> Option<PresetStream> {
    let blob = std::fs::read(captures().join("moved-preset.op21.msgpack.bin")).ok()?;
    let mut wrapped = vec![0u8; 8]; // the 8-byte stream prefix `parse` skips
    wrapped.push(0x83); // fixmap {102: 0, 103: 0, 104: bin}
    wrapped.extend([0x66, 0x00, 0x67, 0x00, 0x68, 0xda]);
    wrapped.extend((blob.len() as u16).to_be_bytes());
    wrapped.extend(&blob);
    PresetStream::parse(&wrapped).ok()
}

fn map_get_mut(v: &mut Value, key: i64) -> Option<&mut Value> {
    match v {
        Value::Map(m) => m
            .iter_mut()
            .find(|(k, _)| k.as_i64() == Some(key))
            .map(|(_, v)| v),
        _ => None,
    }
}

/// Erase the value-preserving serializer quirks (layout zero-key, controller row order — see
/// module doc) so the comparison tests the rewrite's substance.
fn normalize(ps: &mut PresetStream) {
    let preset = ps.preset_mut();
    if let Some(layout) = map_get_mut(preset, 3)
        && let Some(Value::Array(positions)) = map_get_mut(layout, 8)
    {
        for entry in positions.iter_mut().flat_map(|p| match p {
            Value::Array(bindings) => bindings.iter_mut(),
            _ => [].iter_mut(),
        }) {
            if let Some(Value::Map(target)) = map_get_mut(entry, 11) {
                target.retain(|(k, v)| k.as_i64() != Some(2) || v.as_i64() != Some(0));
            }
        }
    }
    if let Some(Value::Array(rows)) = map_get_mut(preset, 4) {
        for row in rows.iter_mut() {
            if let Value::Array(assignments) = row {
                assignments.sort_by_key(|a| map_get(a, 0).and_then(Value::as_i64).unwrap_or(0));
            }
        }
    }
}

fn slot_content_mut(ps: &mut PresetStream, slot: usize) -> &mut Value {
    let group = map_get_mut(ps.preset_mut(), 0).expect("dsp group");
    let Some(Value::Array(slots)) = map_get_mut(group, 22) else {
        panic!("slot array");
    };
    map_get_mut(&mut slots[slot], 20).expect("slot content")
}

fn class_of(ps: &mut PresetStream, slot: usize) -> Option<i64> {
    map_get(slot_content_mut(ps, slot), 9).and_then(Value::as_i64)
}

/// Assert one side's class byte spelling at `slot`, then erase it — the pedal accepts both the
/// device vocabulary (ours) and POD Go Edit's (the oracle's) and normalizes on read-back.
fn pin_class(ps: &mut PresetStream, slot: usize, expect: i64) {
    assert_eq!(class_of(ps, slot), Some(expect), "class at slot {slot}");
    let content = slot_content_mut(ps, slot);
    if let Value::Map(m) = content {
        m.retain(|(k, _)| k.as_i64() != Some(9));
    }
}

/// Assert which IR spelling `slot` holds — the device's read-back (`device: true`: six stored
/// values ending in the device-generated extra, NUL-terminated uuid) or POD Go Edit's
/// (five values, bare uuid) — then reduce both to the common five-value bare form.
fn pin_ir(ps: &mut PresetStream, slot: usize, device: bool) {
    let content = slot_content_mut(ps, slot);
    let uuid = map_get(content, 27)
        .and_then(|v| v.as_str())
        .expect("IR uuid")
        .to_string();
    assert_eq!(
        uuid.ends_with('\0'),
        device,
        "uuid termination at slot {slot}"
    );
    let bank = map_get_mut(content, 11).expect("param bank");
    let stored = map_get(bank, 2).and_then(Value::as_i64);
    let Some(Value::Array(values)) = map_get_mut(bank, 4) else {
        panic!("IR values");
    };
    if device {
        assert_eq!(stored, Some(6), "device IR stores six values");
        assert_eq!(values.pop().and_then(|v| v.as_i64()), Some(6), "the extra");
    } else {
        assert_eq!(stored, Some(5), "POD Go Edit writes five values");
    }
    let five = Value::from(5);
    if let Value::Map(bank) = bank {
        for (k, v) in bank.iter_mut() {
            if k.as_i64() == Some(2) {
                *v = five.clone();
            }
        }
    }
    if let Value::Map(content) = content {
        for (k, v) in content.iter_mut() {
            if k.as_i64() == Some(27) {
                *v = Value::from(uuid.trim_end_matches('\0'));
            }
        }
    }
}

/// Moving slot 1 to slot 10 on the captured "before" reproduces POD Go Edit's own rewrite —
/// every slot, the renumbered footswitch and controller targets, the rotated snapshot matrices,
/// and the selection landing on the destination.
#[test]
fn the_move_reproduces_pod_go_edits_rewrite() {
    let (Some(mut moved), Some(mut oracle)) = (before(), after()) else {
        eprintln!("skipping: needs the POD Go ir2 + move captures");
        return;
    };
    assert!(moved.move_block_single_row(1, 10), "the move must apply");
    // Post-move geography: FX loop at 4, IR at 6, EQ at 7.
    pin_class(&mut moved, 4, 9);
    pin_class(&mut oracle, 4, 8);
    pin_class(&mut moved, 7, 23);
    pin_class(&mut oracle, 7, 1);
    pin_ir(&mut moved, 6, true);
    pin_ir(&mut oracle, 6, false);
    normalize(&mut moved);
    normalize(&mut oracle);
    assert_keys_equal(&moved.preset, &oracle.preset);
}

/// Compare the two preset documents by structure — maps as key → value sets (the two writers
/// order their map keys differently, which carries no meaning), arrays positionally, every leaf
/// exactly. Failures name the differing integer-key paths.
fn assert_keys_equal(a: &Value, b: &Value) {
    fn diff(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
        match (a, b) {
            (Value::Map(x), Value::Map(y)) => {
                let mut keys: Vec<i64> = x
                    .iter()
                    .chain(y.iter())
                    .filter_map(|(k, _)| k.as_i64())
                    .collect();
                keys.sort_unstable();
                keys.dedup();
                for k in keys {
                    match (map_get(a, k), map_get(b, k)) {
                        (Some(va), Some(vb)) => diff(va, vb, &format!("{path}/{k}"), out),
                        (Some(_), None) => out.push(format!("{path}/{k}: present vs absent")),
                        (None, Some(_)) => out.push(format!("{path}/{k}: absent vs present")),
                        _ => {}
                    }
                }
            }
            (Value::Array(x), Value::Array(y)) => {
                if x.len() != y.len() {
                    out.push(format!("{path}: len {} vs {}", x.len(), y.len()));
                }
                for (i, (va, vb)) in x.iter().zip(y).enumerate() {
                    diff(va, vb, &format!("{path}[{i}]"), out);
                }
            }
            _ => {
                if a != b {
                    out.push(format!("{path}: {a:?} vs {b:?}"));
                }
            }
        }
    }
    let mut out = Vec::new();
    diff(a, b, "", &mut out);
    assert!(
        out.is_empty(),
        "the documents differ at {} path(s):\n{}",
        out.len(),
        out.join("\n")
    );
}

/// The inverse move (10 back to 1) on the "after" stream restores the "before" — the
/// rotate-right path, covered against the same two captured streams.
#[test]
fn the_inverse_move_restores_the_original() {
    let (Some(mut original), Some(mut restored)) = (before(), after()) else {
        return;
    };
    assert!(restored.move_block_single_row(10, 1), "the move must apply");
    // The selection is the one asymmetry: the editor leaves it on the moved block, and the
    // original had it on slot 7 — pin ours, then align for the comparison.
    assert_eq!(
        map_get(&restored.preset, 6).and_then(|s| map_get(s, 98)),
        Some(&Value::from(1))
    );
    if let Some(sel) = map_get_mut(original.preset_mut(), 6) {
        *sel = map_get(&restored.preset, 6).unwrap().clone();
    }
    // Pre-move geography: FX loop at 5, IR at 7, EQ at 8.
    pin_class(&mut original, 5, 9);
    pin_class(&mut restored, 5, 8);
    pin_class(&mut original, 8, 23);
    pin_class(&mut restored, 8, 1);
    pin_ir(&mut original, 7, true);
    pin_ir(&mut restored, 7, false);
    normalize(&mut original);
    normalize(&mut restored);
    assert_keys_equal(&restored.preset, &original.preset);
}

/// The guard rails: a no-op move, the input/output nodes, and out-of-range slots all refuse
/// without touching the document.
#[test]
fn the_io_nodes_stay_put() {
    let Some(mut ps) = before() else {
        return;
    };
    let pristine = ps.preset.clone();
    for (src, dst) in [(3, 3), (0, 5), (5, 0), (5, 11), (11, 5), (1, 12)] {
        assert!(
            !ps.move_block_single_row(src, dst),
            "({src},{dst}) must refuse"
        );
    }
    assert_eq!(ps.preset, pristine, "a refused move must change nothing");
}
