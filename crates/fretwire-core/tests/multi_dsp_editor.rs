//! The editor model over a **two-DSP** preset: one `DspView` per DSP, blocks addressed by the
//! global wire slot, and per-DSP load accounting.
//!
//! Like `fretwire-data`'s `multi_dsp.rs`, this builds a synthetic two-DSP preset rather than
//! shipping a Helix Floor capture (those are contributor-supplied and hold personal presets — see
//! `.gitignore`). The model indices are real ones so names and params resolve against the catalog.
//!
//! Needs the (unshipped) Line 6 reference data, so the whole file compiles only when a local copy
//! is present (`have_bundled_data`, set by build.rs).
#![cfg(have_bundled_data)]

use fretwire_core::editor::Catalog;
use fretwire_data::stream::{DSP_SLOT_STRIDE, slot_kind};
use rmpv::Value;

fn catalog() -> Catalog {
    Catalog::from_data_dir(&fretwire_core::data_dir()).unwrap()
}

fn slot(kind: i64, content: Value) -> Value {
    Value::Map(vec![
        (Value::from(19), Value::from(kind)),
        (Value::from(20), content),
    ])
}

fn params(values: &[f32]) -> Value {
    Value::Array(values.iter().map(|v| Value::F32(*v)).collect())
}

/// Effect block: model at `24 → 25`, params at `11 → 4`.
fn effect(model: i64, values: &[f32]) -> Value {
    slot(
        slot_kind::EFFECT,
        Value::Map(vec![
            (
                Value::from(24),
                Value::Map(vec![
                    (Value::from(25), Value::from(model)),
                    (Value::from(26), Value::from(-1)),
                ]),
            ),
            (Value::from(10), Value::from(true)),
            (
                Value::from(11),
                Value::Map(vec![(Value::from(4), params(values))]),
            ),
        ]),
    )
}

/// Looper block: model at content key `8`, params at `7 → 4`.
fn looper(model: i64, values: &[f32]) -> Value {
    slot(
        slot_kind::LOOPER,
        Value::Map(vec![
            (Value::from(8), Value::from(model)),
            (Value::from(10), Value::from(true)),
            (
                Value::from(7),
                Value::Map(vec![(Value::from(4), params(values))]),
            ),
        ]),
    )
}

fn node(kind: i64, model: i64, pos: i64) -> Value {
    let holder = Value::Map(vec![
        (Value::from(8), Value::from(model)),
        (Value::from(13), Value::from(pos)),
        (Value::from(10), Value::from(true)),
        (
            Value::from(7),
            Value::Map(vec![(Value::from(4), params(&[0.5, 0.0]))]),
        ),
    ]);
    slot(kind, Value::Map(vec![(Value::from(15), holder)]))
}

fn dsp_group(split_type: i64, blocks: Vec<(usize, Value)>) -> Value {
    let mut slots = vec![slot(slot_kind::EMPTY, Value::Nil); DSP_SLOT_STRIDE as usize];
    slots[0] = node(0, 900, 0);
    slots[9] = node(1, 901, 9);
    slots[10] = node(slot_kind::SPLIT, 257, 5);
    slots[19] = node(slot_kind::MIXER, 151, 7);
    for (i, b) in blocks {
        slots[i] = b;
    }
    Value::Map(vec![
        (Value::from(21), Value::from(split_type)),
        (Value::from(22), Value::Array(slots)),
    ])
}

/// DSP1: Volume @1, Hall @8. DSP2: a Looper @7 and a Cali Rectifire @13 (row B).
fn two_dsp_stream() -> Vec<u8> {
    let dsp0 = dsp_group(
        0,
        vec![
            (1, effect(261, &[1.0, 0.0])),
            (8, effect(243, &[0.8, 0.1, 83.0, 4300.0, 0.3, 0.0])),
        ],
    );
    let dsp1 = dsp_group(
        3,
        vec![
            (7, looper(153, &[0.5, 1.0])),
            (
                13,
                effect(18, &[0.68, 0.45, 0.4, 0.66, 0.56, 0.57, 0.35, 0.4]),
            ),
        ],
    );
    let preset = Value::Map(vec![
        (Value::from(0), dsp0),
        (Value::from(1), dsp1),
        (
            Value::from(7),
            Value::Map(vec![(Value::from(36), Value::from("P21\0"))]),
        ),
    ]);

    let mut blob = Vec::new();
    rmpv::encode::write_value(&mut blob, &Value::from("l6-helix\0")).unwrap();
    rmpv::encode::write_value(&mut blob, &Value::from("hdr")).unwrap();
    rmpv::encode::write_value(&mut blob, &preset).unwrap();

    let envelope = Value::Map(vec![(Value::from(104), Value::Binary(blob))]);
    let mut out = vec![0u8; 8];
    rmpv::encode::write_value(&mut out, &envelope).unwrap();
    out
}

#[test]
fn both_dsps_load_with_resolved_names_and_global_slots() {
    let p = catalog().load_preset(&two_dsp_stream()).unwrap();
    assert_eq!(p.device_model.as_deref(), Some("P21"));

    let by_slot: Vec<(i64, usize, &str)> = p
        .blocks
        .iter()
        .map(|b| (b.slot, b.dsp, b.model_name.as_str()))
        .collect();
    assert_eq!(
        by_slot,
        vec![
            (1, 0, "Volume"),
            (8, 0, "Hall"),
            (27, 1, "6 Switch Looper"),
            (33, 1, "Cali Rectifire"),
        ],
        "DSP2 blocks appear at their global slots (20 + index)"
    );

    // The Looper resolved its model through the type-7 content shape, so it has real params.
    let looper = p.block(27).expect("looper at global slot 27");
    assert_eq!(looper.dsp, 1);
    assert_eq!(
        looper.params.first().map(|x| x.name.as_str()),
        Some("Playback")
    );

    // A DSP2 block's params resolve in the model's own Helix.sym order.
    let amp = p.block(33).expect("amp at global slot 33");
    assert_eq!(amp.params[2].name, "Mid");
}

#[test]
fn one_dsp_view_per_dsp_with_its_own_split_state() {
    let p = catalog().load_preset(&two_dsp_stream()).unwrap();
    assert_eq!(p.dsps.len(), 2);
    assert_eq!(p.dsps.iter().map(|d| d.dsp).collect::<Vec<_>>(), vec![0, 1]);

    // DSP1 is serial (group key 21 = 0), DSP2 is split (3) — a state the old preset-wide flag
    // could not represent.
    assert!(!p.dsps[0].split);
    assert!(p.dsps[1].split);
    assert!(p.split(), "the preset counts as split when any DSP is");

    // Only the split DSP exposes routing nodes.
    assert!(p.dsps[0].split_node.is_none());
    let split1 = p.dsps[1].split_node.as_ref().expect("DSP2 split node");
    assert_eq!(split1.slot, 30, "DSP2's split node is at wire slot 20 + 10");
    assert_eq!(p.dsps[1].mixer_node.as_ref().unwrap().slot, 39);

    // Each DSP has its own input/output nodes, at its own global slots.
    assert_eq!(p.dsps[0].input_node.as_ref().unwrap().slot, 0);
    assert_eq!(p.dsps[0].output_node.as_ref().unwrap().slot, 9);
    assert_eq!(p.dsps[1].input_node.as_ref().unwrap().slot, 20);
    assert_eq!(p.dsps[1].output_node.as_ref().unwrap().slot, 29);

    // The bare accessors keep meaning DSP 0, which is what a one-DSP device has.
    assert_eq!(p.input_node().map(|n| n.slot), Some(0));
    assert!(p.split_node().is_none());
}

#[test]
fn routing_nodes_of_every_dsp_are_reachable_by_slot() {
    let p = catalog().load_preset(&two_dsp_stream()).unwrap();
    // `block()` spans blocks plus every DSP's routing nodes.
    assert!(p.block(30).is_some(), "DSP2's split node");
    assert!(p.is_split_node(30));
    assert!(p.is_mixer_node(39));
    assert!(
        !p.is_split_node(10),
        "DSP1 is serial — it has no exposed split node"
    );
    assert!(p.block(20).is_some(), "DSP2's input node");
}

#[test]
fn dsp_load_is_reported_per_dsp() {
    let p = catalog().load_preset(&two_dsp_stream()).unwrap();
    let loads = p.dsp_load_by_dsp();
    assert_eq!(loads.len(), 2);
    assert_eq!(
        loads.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
        vec![0, 1]
    );
    for (dsp, load) in &loads {
        assert!(*load > 0.0, "dsp {dsp} should draw some load");
    }
    // Each DSP is budgeted on its own; the flat total is merely their sum.
    let total: f64 = loads.iter().map(|(_, l)| l).sum();
    assert!((total - p.dsp_load).abs() < 1e-6);
    for (dsp, load) in &loads {
        assert!((p.dsp_load_on(*dsp) - load).abs() < 1e-6);
    }
}

/// Headroom is measured against the load the pedal actually stops at (~75), not the 100 the
/// percentage implies. Reading it as `100 - load` is what told the tester he had 27% free on a
/// preset that would not take another block — see `editor::DSP_CEILING`.
#[test]
fn free_dsp_is_measured_against_the_ceiling_not_a_hundred() {
    use fretwire_core::editor::DSP_CEILING;
    let p = catalog().load_preset(&two_dsp_stream()).unwrap();

    for (dsp, load) in p.dsp_load_by_dsp() {
        let free = p.dsp_free_on(dsp);
        assert!((free - (DSP_CEILING - load)).abs() < 1e-6);
        assert!(
            free < 100.0 - load,
            "dsp {dsp}: free must be stingier than the naive 100 - {load}"
        );
    }
    assert_eq!(
        p.dsp_free_by_dsp(),
        p.dsp_load_by_dsp()
            .iter()
            .map(|(d, _)| (*d, p.dsp_free_on(*d)))
            .collect::<Vec<_>>()
    );

    // A DSP loaded past the ceiling reports no room rather than a negative figure, and an unused
    // DSP index reports the whole ceiling rather than panicking.
    assert_eq!(p.dsp_free_on(97), DSP_CEILING);
}

#[test]
fn grid_partitions_by_dsp_and_keeps_slots_global() {
    let p = catalog().load_preset(&two_dsp_stream()).unwrap();
    let grid = p.grid();
    assert_eq!(
        grid.len(),
        p.dsps.iter().map(|d| d.grid.len()).sum::<usize>()
    );

    for c in &grid {
        let expected_dsp = (c.slot / DSP_SLOT_STRIDE) as usize;
        assert_eq!(
            c.dsp, expected_dsp,
            "cell {} is tagged with the wrong dsp",
            c.slot
        );
    }
    // Same (row, column) on different DSPs is legal — they are distinct cells.
    let collisions = grid
        .iter()
        .filter(|c| c.row == 0 && c.column == 1)
        .map(|c| (c.dsp, c.slot))
        .collect::<Vec<_>>();
    assert_eq!(collisions, vec![(0, 1), (1, 21)]);
}

#[test]
fn edits_to_a_dsp2_block_address_it_by_the_global_slot() {
    let p = catalog().load_preset(&two_dsp_stream()).unwrap();
    let amp = p.block(33).expect("Cali Rectifire on DSP2");

    // The edit builders take a single slot integer and need no DSP field — the whole point of the
    // global numbering. Byte-compare against the builder called with the raw slot.
    let from_block = amp.set_param_edit(2, 0.41, 7);
    let direct = fretwire_protocol::edit::set_value(33, 2, 0.41, 7);
    assert_eq!(from_block, direct);

    let bypass_from_block = amp.set_enabled_edit(false, 8);
    assert_eq!(
        bypass_from_block,
        fretwire_protocol::edit::bypass(33, false, 8)
    );
}
