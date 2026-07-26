//! Multi-DSP preset traversal: the second slot array (preset key `1`), the **global** wire slot
//! numbering that spans both DSPs, and the Looper (`type 7`) slot shape.
//!
//! The Helix Floor captures that established all of this are contributor-supplied and hold their
//! personal presets, so they are not in the repo (see `.gitignore`). These tests therefore build a
//! **synthetic** two-DSP preset with the structure the captures documented — our own bytes, safe to
//! commit, and exercising exactly the traversal logic. The corresponding real-device evidence is
//! recorded in `docs/helix-floor.md` and `docs/preset-format.md`.
//!
//! Needs no Line 6 reference data (nothing here resolves a model name), so it runs on a clean clone.

use fretwire_data::stream::{DSP_SLOT_STRIDE, PresetStream, slot_kind, split_wire_slot, wire_slot};
use rmpv::Value;

/// `{19: kind, 20: content}` — one entry of a DSP's 20-slot array.
fn slot(kind: i64, content: Value) -> Value {
    Value::Map(vec![
        (Value::from(19), Value::from(kind)),
        (Value::from(20), content),
    ])
}

/// An empty slot (kind 8, nil content).
fn empty() -> Value {
    slot(slot_kind::EMPTY, Value::Nil)
}

/// A populated **effect** block (kind 6): model at `24 → 25`, params at `11 → 4`, enabled at `10`.
fn effect(model: i64, enabled: bool, params: &[f32]) -> Value {
    let content = Value::Map(vec![
        (
            Value::from(24),
            Value::Map(vec![
                (Value::from(25), Value::from(model)),
                (Value::from(26), Value::from(-1)), // no paired cab
            ]),
        ),
        (Value::from(10), Value::from(enabled)),
        (
            Value::from(11),
            Value::Map(vec![(
                Value::from(4),
                Value::Array(params.iter().map(|v| Value::F32(*v)).collect()),
            )]),
        ),
    ]);
    slot(slot_kind::EFFECT, content)
}

/// A populated **Looper** block (kind 7) — the different content shape: model index directly at
/// content key `8` (not `24 → 25`), params at `7 → 4` (not `11 → 4`), enabled at `10` (same).
fn looper(model: i64, enabled: bool, params: &[f32]) -> Value {
    let content = Value::Map(vec![
        (Value::from(8), Value::from(model)),
        (Value::from(10), Value::from(enabled)),
        (
            Value::from(7),
            Value::Map(vec![(
                Value::from(4),
                Value::Array(params.iter().map(|v| Value::F32(*v)).collect()),
            )]),
        ),
        (Value::from(9), Value::from(1)), // the secondary id/flag the device carries
    ]);
    slot(slot_kind::LOOPER, content)
}

/// A structural node (kinds 0/1/2/3): model + position live in a content sub-map that carries key
/// `8`, exactly as the real presets encode them.
fn node(kind: i64, model: i64, pos: i64) -> Value {
    let holder = Value::Map(vec![
        (Value::from(8), Value::from(model)),
        (Value::from(13), Value::from(pos)),
        (Value::from(10), Value::from(true)),
        (
            Value::from(7),
            Value::Map(vec![(
                Value::from(4),
                Value::Array(vec![Value::F32(0.5), Value::F32(0.0)]),
            )]),
        ),
    ]);
    slot(kind, Value::Map(vec![(Value::from(15), holder)]))
}

/// Build one DSP group: `{21: split_type, 22: Array[20]}`.
///
/// Layout mirrors the device's fixed topology — index 0 = input, 1..8 = path A, 9 = output-A node,
/// 10 = split node, 11..18 = path B, 19 = mixer node.
fn dsp_group(split_type: i64, blocks: Vec<(usize, Value)>) -> Value {
    let mut slots = vec![empty(); DSP_SLOT_STRIDE as usize];
    slots[0] = node(0, 900, 0); // input
    slots[9] = node(1, 901, 9); // output
    slots[10] = node(slot_kind::SPLIT, 257, 5); // split, at column 5
    slots[19] = node(slot_kind::MIXER, 151, 7); // mixer, at column 7
    for (i, b) in blocks {
        slots[i] = b;
    }
    Value::Map(vec![
        (Value::from(21), Value::from(split_type)),
        (Value::from(22), Value::Array(slots)),
    ])
}

/// Wrap a preset map in the stream framing the device uses: an 8-byte transport header, then the
/// envelope map `{104: <blob>}` whose blob is `magic ⧺ header ⧺ preset-map`.
fn stream(preset: Value) -> Vec<u8> {
    let mut blob = Vec::new();
    rmpv::encode::write_value(&mut blob, &Value::from("l6-helix\0")).unwrap();
    rmpv::encode::write_value(&mut blob, &Value::from("hdr")).unwrap();
    rmpv::encode::write_value(&mut blob, &preset).unwrap();

    let envelope = Value::Map(vec![(Value::from(104), Value::Binary(blob))]);
    let mut out = vec![0u8; 8];
    rmpv::encode::write_value(&mut out, &envelope).unwrap();
    out
}

/// A two-DSP preset shaped like the Helix Floor's "Pull Me Under":
/// - DSP1: two path-A blocks, one path-B block, split type 2.
/// - DSP2: a Looper and an effect on path A, two effects on path B, split type 3.
///
/// Crucially both DSPs have a block at **index 13**, which is the case a per-DSP numbering scheme
/// cannot address unambiguously.
fn two_dsp_stream() -> Vec<u8> {
    let dsp0 = dsp_group(
        2,
        vec![
            (1, effect(261, true, &[1.0])),
            (5, effect(25, true, &[0.24, 0.5])),
            (13, effect(101, false, &[0.05, 0.5, 0.5])),
        ],
    );
    let dsp1 = dsp_group(
        3,
        vec![
            (7, looper(153, true, &[0.5, 1.0])),
            (8, effect(243, true, &[0.8, 0.1, 83.0])),
            (13, effect(18, true, &[0.68, 0.45, 0.4])),
            (17, effect(80, true, &[0.6, 0.19])),
        ],
    );
    stream(Value::Map(vec![
        (Value::from(0), dsp0),
        (Value::from(1), dsp1),
        (
            Value::from(7),
            Value::Map(vec![(Value::from(36), Value::from("P21\0"))]),
        ),
    ]))
}

/// A single-DSP preset — key `1` present but nil, exactly as the HX Stomp sends it.
fn one_dsp_stream() -> Vec<u8> {
    let dsp0 = dsp_group(
        0,
        vec![
            (1, effect(261, true, &[1.0])),
            (5, effect(25, true, &[0.24])),
        ],
    );
    stream(Value::Map(vec![
        (Value::from(0), dsp0),
        (Value::from(1), Value::Nil),
        (
            Value::from(7),
            Value::Map(vec![(Value::from(36), Value::from("P33\0"))]),
        ),
    ]))
}

fn parse(s: &[u8]) -> PresetStream {
    PresetStream::parse(s).expect("synthetic stream should parse")
}

#[test]
fn wire_slot_round_trips() {
    for dsp in 0..2usize {
        for index in 0..DSP_SLOT_STRIDE as usize {
            let s = wire_slot(dsp, index);
            assert_eq!(
                split_wire_slot(s),
                (dsp, index),
                "round trip for dsp {dsp} index {index}"
            );
        }
    }
    // The documented framing: DSP1 = 0..19, DSP2 = 20..39.
    assert_eq!(wire_slot(0, 0), 0);
    assert_eq!(wire_slot(0, 19), 19);
    assert_eq!(wire_slot(1, 0), 20);
    assert_eq!(wire_slot(1, 19), 39);
}

#[test]
fn single_dsp_preset_is_unchanged_by_the_second_group() {
    let ps = parse(&one_dsp_stream());
    assert_eq!(ps.dsps(), vec![0], "key 1 is nil — only DSP 0 is populated");
    let blocks = ps.effect_blocks();
    assert_eq!(blocks.len(), 2);
    // Every slot stays below the stride, i.e. numerically identical to the old per-DSP indexing.
    for b in &blocks {
        assert_eq!(b.dsp, 0);
        assert!(
            b.wire_slot() < DSP_SLOT_STRIDE,
            "slot {} should be DSP 0",
            b.wire_slot()
        );
        assert_eq!(b.wire_slot(), b.index as i64);
    }
    assert!(!ps.dsp_is_split(0), "split type 0 = serial");
    assert!(!ps.is_split());
}

#[test]
fn both_dsp_groups_are_walked() {
    let ps = parse(&two_dsp_stream());
    assert_eq!(ps.dsps(), vec![0, 1]);

    let blocks = ps.effect_blocks();
    assert_eq!(blocks.len(), 7, "3 on DSP1 + 4 on DSP2 (the Looper counts)");
    assert_eq!(blocks.iter().filter(|b| b.dsp == 0).count(), 3);
    assert_eq!(blocks.iter().filter(|b| b.dsp == 1).count(), 4);

    // Reading only key 0 — the old behaviour — would have found just the first three.
    assert_eq!(ps.dsp_blocks(0).iter().filter(|b| b.is_block()).count(), 3);
}

#[test]
fn dsp2_blocks_get_global_wire_slots() {
    let ps = parse(&two_dsp_stream());
    let slots: Vec<i64> = ps.effect_blocks().iter().map(|b| b.wire_slot()).collect();
    // DSP1 indices 1, 5, 13 → 1, 5, 13. DSP2 indices 7, 8, 13, 17 → 27, 28, 33, 37.
    assert_eq!(slots, vec![1, 5, 13, 27, 28, 33, 37]);

    // The whole point: two blocks at the same per-DSP index are distinguishable.
    let at_13: Vec<i64> = ps
        .effect_blocks()
        .iter()
        .filter(|b| b.index == 13)
        .map(|b| b.wire_slot())
        .collect();
    assert_eq!(
        at_13,
        vec![13, 33],
        "index 13 exists on both DSPs and must not collide"
    );
}

#[test]
fn loaded_blocks_carry_the_global_slot_and_its_dsp() {
    let ps = parse(&two_dsp_stream());
    let loaded = ps.loaded_blocks();
    assert_eq!(loaded.len(), 7);
    for b in &loaded {
        let (dsp, index) = split_wire_slot(b.slot);
        assert_eq!(dsp, b.dsp, "slot {} should decode to dsp {}", b.slot, b.dsp);
        assert!(index < DSP_SLOT_STRIDE as usize);
    }
    // Row B is resolved against *each DSP's own* split node (index 10), not a preset-wide one.
    let rows: Vec<(i64, u8)> = loaded.iter().map(|b| (b.slot, b.row)).collect();
    assert_eq!(
        rows,
        vec![(1, 0), (5, 0), (13, 1), (27, 0), (28, 0), (33, 1), (37, 1)]
    );
}

#[test]
fn looper_slots_are_enumerated_with_their_own_content_shape() {
    let ps = parse(&two_dsp_stream());
    let loop_block = ps
        .effect_blocks()
        .into_iter()
        .find(|b| b.kind == slot_kind::LOOPER)
        .expect("the type-7 Looper should be enumerated, not skipped");

    assert_eq!(loop_block.dsp, 1);
    assert_eq!(loop_block.wire_slot(), 27);
    // Model index comes from content key `8`, *not* `24 → 25` (which a Looper doesn't have).
    assert_eq!(loop_block.model_ref, Some(153));
    // Params come from `7 → 4`, not `11 → 4`.
    assert_eq!(loop_block.params.len(), 2);
    // Enabled is the same key (`10`) as an effect block.
    assert_eq!(loop_block.bypassed, Some(false));
    assert!(
        loop_block.paired_ref.is_none(),
        "a Looper never has a paired model"
    );

    // An effect block on the same DSP still uses the type-6 shape.
    let fx = ps
        .effect_blocks()
        .into_iter()
        .find(|b| b.wire_slot() == 28)
        .unwrap();
    assert_eq!(fx.model_ref, Some(243));
    assert_eq!(fx.params.len(), 3);
}

#[test]
fn each_dsp_has_its_own_split_state_and_nodes() {
    let ps = parse(&two_dsp_stream());
    // Non-zero group key 21 = split, and the value is the split *type* — 2 and 3 here, neither of
    // which the old `== 1` test would have recognised.
    assert!(ps.dsp_is_split(0));
    assert!(ps.dsp_is_split(1));

    let split0 = ps
        .dsp_structural_node(0, slot_kind::SPLIT)
        .expect("DSP1 split node");
    let split1 = ps
        .dsp_structural_node(1, slot_kind::SPLIT)
        .expect("DSP2 split node");
    assert_eq!(split0.slot, 10, "DSP1's split node is at wire slot 10");
    assert_eq!(split1.slot, 30, "DSP2's split node is at wire slot 30");
    assert_eq!(split0.dsp, 0);
    assert_eq!(split1.dsp, 1);

    // Same for the fixed I/O nodes.
    assert_eq!(ps.dsp_io_node(0, 0).unwrap().slot, 0);
    assert_eq!(ps.dsp_io_node(1, 0).unwrap().slot, 20);
    assert_eq!(ps.dsp_io_node(1, 1).unwrap().slot, 29);

    // The no-DSP-argument accessors keep meaning DSP 0.
    assert_eq!(ps.structural_node(slot_kind::SPLIT).unwrap().slot, 10);
    assert_eq!(ps.io_node(0).unwrap().slot, 0);
    assert_eq!(ps.structural_node_pos(slot_kind::SPLIT), Some(5));
}

#[test]
fn grid_cells_are_tagged_with_their_dsp_and_global_slot() {
    let ps = parse(&two_dsp_stream());
    let grid = ps.grid();
    assert!(grid.iter().any(|c| c.dsp == 0));
    assert!(grid.iter().any(|c| c.dsp == 1));

    for c in &grid {
        assert_eq!(
            split_wire_slot(c.slot).0,
            c.dsp,
            "cell slot must decode to its own dsp"
        );
    }
    // Per-DSP grids partition the whole grid.
    assert_eq!(ps.dsp_grid(0).len() + ps.dsp_grid(1).len(), grid.len());

    // A row-B cell's column is its index within its own DSP's row B: local slot 13 is row-B
    // column 3, in the same 1..=8 column space the top row uses.
    let b33 = grid
        .iter()
        .find(|c| c.slot == 33)
        .expect("DSP2 index 13 is a cell");
    assert_eq!((b33.dsp, b33.row, b33.occupied), (1, 1, true));
    assert_eq!(b33.column, 13 - 10);

    // The Looper occupies its cell like any other block.
    let looper_cell = grid.iter().find(|c| c.slot == 27).unwrap();
    assert!(
        looper_cell.occupied,
        "a type-7 slot is occupied, not an empty cell"
    );
}

#[test]
fn set_slot_empty_addresses_by_global_slot() {
    let mut ps = parse(&two_dsp_stream());
    assert!(ps.set_slot_empty(33), "clear DSP2's block at index 13");

    let slots: Vec<i64> = ps.effect_blocks().iter().map(|b| b.wire_slot()).collect();
    assert_eq!(slots, vec![1, 5, 13, 27, 28, 37], "only slot 33 is gone");
    assert!(
        ps.effect_blocks().iter().any(|b| b.wire_slot() == 13),
        "DSP1's index-13 block must be untouched"
    );

    // Out of range for the two groups we have.
    assert!(!ps.set_slot_empty(99));
}

#[test]
fn re_serializing_preserves_both_dsp_groups() {
    let ps = parse(&two_dsp_stream());
    let blob = ps.to_blob();

    // Re-wrap the round-tripped blob and parse it again.
    let envelope = Value::Map(vec![(Value::from(104), Value::Binary(blob))]);
    let mut out = vec![0u8; 8];
    rmpv::encode::write_value(&mut out, &envelope).unwrap();

    let again = parse(&out);
    assert_eq!(again.dsps(), vec![0, 1]);
    assert_eq!(again.effect_blocks(), ps.effect_blocks());
    assert_eq!(again.grid(), ps.grid());
}
