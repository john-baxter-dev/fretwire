//! `Helix.sym` gives the device's authoritative per-model param ordering (Mono/Stereo variants).
//! These tests confirm we pick the right variant from a preset block's value-vector length, and
//! that doing so fixes the param mislabeling the `.models` order produced for mono blocks.
//!
//! Needs the unshipped Line 6 reference data; compiles only with a local copy present
//! (`have_bundled_data`, set by build.rs).
#![cfg(have_bundled_data)]

use fretwire_data::stream::PresetStream;
use fretwire_data::symbols::DeviceSymbols;
use std::path::PathBuf;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(data_dir().join(name)).unwrap()
}

// Our own captured preset streams live in the in-repo `captures/` dir (not Line 6 data, so not
// part of the import cache).
fn capture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures").join(name);
    std::fs::read(p).unwrap()
}

// Line 6 reference data from the `fretwire import-data` cache (see build.rs / fretwire_core::data_dir).
fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FRETWIRE_DATA_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("fretwire").join("data")
}

fn syms() -> DeviceSymbols {
    DeviceSymbols::parse(&data("Helix.sym")).unwrap()
}

#[test]
fn parses_device_symbols() {
    let s = syms();
    assert!(s.len() > 800, "expected ~833 device symbols, got {}", s.len());
    // Device symbols carry a Mono/Stereo suffix and differ in order/length.
    let mono = s.params("HD2_TremoloHarmonicMono").unwrap();
    let stereo = s.params("HD2_TremoloHarmonicStereo").unwrap();
    assert_eq!(mono.len(), 10);
    assert_eq!(stereo.len(), 11);
    assert!(mono.contains(&"Mix".to_string()));
    assert!(!mono.contains(&"Spread".to_string()), "mono tremolo has no Spread");
    assert!(stereo.contains(&"Spread".to_string()), "stereo tremolo has Spread");
}

#[test]
fn resolves_variant_by_vector_length() {
    let s = syms();
    // (host symbolic id, preset device-vector length) -> expected variant.
    let cases = [
        ("HD2_TremoloHarmonic", 10, "Mono"),
        ("HD2_Chorus70sChorus", 11, "Mono"),
        ("HD2_DelayBucketBrigade", 9, "Stereo"),
    ];
    for (host, count, want) in cases {
        let (variant, params) = s
            .resolve_order(host, count)
            .unwrap_or_else(|| panic!("{host} ({count}) did not resolve"));
        assert_eq!(variant, want, "{host}");
        assert_eq!(params.len(), count);
    }
}

#[test]
fn device_order_fixes_harmonic_tremolo_mislabel() {
    // The preset's Harmonic Tremolo is a 10-value MONO block. The `.models` order would put
    // `Spread` at index 8; the device (Mono) order has no Spread — index 8 is `SyncSelect1`,
    // whose value 6 matches its default. This is the misalignment device order resolves.
    use fretwire_data::stream::{ParamValue, PresetStream};
    let s = syms();
    let ps = PresetStream::parse(&capture("preset1_stream.msgpack.bin")).unwrap();

    let ht = ps
        .footswitch_layout()
        .into_iter()
        .flatten()
        .find(|p| p.model_name == "Harmonic Tremolo")
        .unwrap();
    let blocks = ps.blocks();
    let vals = &blocks.iter().find(|b| b.index as i64 == ht.slot.unwrap()).unwrap().params;

    let (variant, order) = s.resolve_order("HD2_TremoloHarmonic", vals.len()).unwrap();
    assert_eq!(variant, "Mono");
    // Index 8 in device (Mono) order is SyncSelect1, not Spread.
    assert_eq!(order[8], "SyncSelect1");
    assert_eq!(vals[8], ParamValue::Int(6)); // SyncSelect1 = 6 (its default)
}

#[test]
fn reverb_extra_value_is_named_trails() {
    // VIC_ReverbRotating's symbol lists 12 params, but the preset's Dynamic Hall block carries 13
    // values — the extra trailing value is the `Trails` switch. resolve_order accepts symbol+1 and
    // names it "Trails".
    let s = syms();
    assert_eq!(s.params("VIC_ReverbRotatingMono").unwrap().len(), 12);

    let ps = PresetStream::parse(&capture("preset1_stream.msgpack.bin")).unwrap();
    let dh = ps
        .footswitch_layout()
        .into_iter()
        .flatten()
        .find(|p| p.model_name == "Dynamic Hall")
        .unwrap();
    let blocks = ps.blocks();
    let n = blocks.iter().find(|b| b.index as i64 == dh.slot.unwrap()).unwrap().params.len();
    assert_eq!(n, 13);
    let (_variant, names) = s.resolve_order("VIC_ReverbRotating", n).expect("reverb +1 should match");
    assert_eq!(names.len(), 13);
    assert_eq!(names[0], "Decay");
    assert_eq!(names[12], "Trails");
}

// The parallel routing nodes: the split-type indices decoded from cycle_through_split_types.pcapng
// (op 40 → 256/258/563 on the split slot) must resolve to the expected flow-split symbols, and the
// mixer/join model must expose the A/B level/pan params in the order the join capture edited them.
#[test]
fn split_and_mixer_node_symbols() {
    let s = syms();
    assert_eq!(s.by_index(257).unwrap().0, "HD2_AppDSPFlowSplitY");
    assert_eq!(s.by_index(256).unwrap().0, "HD2_AppDSPFlowSplitAB");
    assert_eq!(s.by_index(258).unwrap().0, "HD2_AppDSPFlowSplitXOver");
    assert_eq!(s.by_index(563).unwrap().0, "HD2_AppDSPFlowSplitDyn");
    // Mixer/join (index 151): params 0..=3 are A Level, A Pan, B Level, B Pan (the join capture
    // edited param indices 0,1,2,3 on the mixer slot).
    let (sym, params) = s.by_index(151).unwrap();
    assert_eq!(sym, "HD2_AppDSPFlowJoin");
    assert_eq!(&params[0..4], &["A Level", "A Pan", "B Level", "B Pan"]);
}

// Parse a real split preset (extracted from move_simple_eq_to_parallel_path.pcapng): the split node
// must resolve to Split Y (257) and the mixer to the join model (151) with its 6 A/B params — via
// `structural_node`, which reads a routing node's model+params from the content sub-map (not key 24).
#[test]
fn structural_node_parses_split_and_mixer() {
    let ps = PresetStream::parse(&capture("split_preset_stream.msgpack.bin")).unwrap();
    assert!(ps.is_split());
    let split = ps.structural_node(fretwire_data::stream::slot_kind::SPLIT).expect("split node");
    assert_eq!(split.model_index, Some(257), "default split type is Split Y");
    assert_eq!(split.params.len(), 3, "Split Y has BalanceA, BalanceB, bypass");
    let mixer = ps.structural_node(fretwire_data::stream::slot_kind::MIXER).expect("mixer node");
    assert_eq!(mixer.model_index, Some(151), "mixer is HD2_AppDSPFlowJoin");
    assert_eq!(mixer.params.len(), 6, "join has A/B level+pan, B polarity, level");
}

// The routing grid maps every draggable slot to exactly one (row, column) cell: top-row cells carry
// `column == slot`, row-B cells align under their A column (`slot − split_idx + 1`), and the input
// slot and split/mixer nodes are excluded. Every effect block shows up as an occupied cell.
#[test]
fn grid_maps_slots_to_rows_and_columns() {
    use fretwire_data::stream::slot_kind;
    let ps = PresetStream::parse(&capture("split_preset_stream.msgpack.bin")).unwrap();
    let blocks = ps.blocks();
    let split_idx =
        blocks.iter().find(|b| b.kind == slot_kind::SPLIT).map(|b| b.index).unwrap() as i64;
    let cells = ps.grid();
    assert!(!cells.is_empty(), "split preset has grid cells");
    assert!(cells.iter().all(|c| c.slot != 0), "input slot is not a cell");
    for c in &cells {
        if c.row == 0 {
            assert_eq!(c.column, c.slot, "top cell column == slot");
            assert!(c.slot < split_idx, "top cells are before the split node");
        } else {
            assert_eq!(c.row, 1, "only rows 0 and 1 exist");
            assert_eq!(c.column, c.slot - split_idx + 1, "row-B column aligns under A");
            assert!(c.slot > split_idx, "row-B cells are after the split node");
        }
    }
    for b in blocks.iter().filter(|b| b.kind == slot_kind::EFFECT) {
        let cell = cells.iter().find(|c| c.slot == b.index as i64).expect("effect block has a cell");
        assert!(cell.occupied, "effect block's cell is occupied");
    }
}

// The split/mixer node slots exist in the fixed 20-slot array even on a **serial** preset (kinds 2/3
// at slots 10/19, with `is_split() == false`) — so the grid always includes the empty row-B cells.
// That's what lets the UI offer "drag to the B row to create the split": the drop is one op-43 into
// an empty 11–18 slot, and the device activates the split itself. [solid — preset1 fixture]
#[test]
fn serial_preset_grid_still_has_row_b_cells() {
    use fretwire_data::stream::slot_kind;
    let ps = PresetStream::parse(&capture("preset1_stream.msgpack.bin")).unwrap();
    assert!(!ps.is_split(), "preset1 is serial");
    let blocks = ps.blocks();
    assert!(
        blocks.iter().any(|b| b.kind == slot_kind::SPLIT)
            && blocks.iter().any(|b| b.kind == slot_kind::MIXER),
        "split/mixer node slots are present even when serial"
    );
    let b_cells: Vec<_> = ps.grid().into_iter().filter(|c| c.row == 1).collect();
    assert_eq!(b_cells.len(), 8, "row B is the full 11–18 slot window");
    assert!(b_cells.iter().all(|c| !c.occupied), "row B is empty on a serial preset");
}

// The fixed input (kind 0, slot 0) / output (kind 1, slot 9) nodes carry their preset-side params
// directly in content `7 → 4` — input [noiseGate, threshold, decay], output [pan, gain] — the
// leading entries of the device symbol's wire order. Edited with plain set-value on the node's slot
// (input-gate capture: op 30 {98:0, 28:0, 119:bool}). [solid]
#[test]
fn io_nodes_expose_their_params() {
    let ps = PresetStream::parse(&capture("preset1_stream.msgpack.bin")).unwrap();
    let input = ps.io_node(0).expect("input node");
    assert_eq!(input.slot, 0);
    assert_eq!(input.params.len(), 3, "gate, threshold, decay");
    assert_eq!(input.params[0], fretwire_data::stream::ParamValue::Bool(false), "gate off");
    assert_eq!(input.params[1], fretwire_data::stream::ParamValue::Float(-48.0), "threshold");
    let output = ps.io_node(1).expect("output node");
    assert_eq!(output.slot, 9);
    assert_eq!(output.params.len(), 2, "pan, gain");
    assert_eq!(output.params[0], fretwire_data::stream::ParamValue::Float(0.5), "pan centered");
    assert!(ps.io_node(6).is_none(), "only kinds 0/1 are io nodes");
}

// Moving the split/join points = writing the node holder's key 13 (`set_node_pos`), then an op-21
// whole-preset write. Round-trip: mutate → to_blob → re-parse → the new positions read back and
// nothing else changed (blocks, grid, split flag).
#[test]
fn set_node_pos_round_trips_through_blob() {
    use fretwire_data::stream::slot_kind;
    let mut ps = PresetStream::parse(&capture("dual_amp_stream.msgpack.bin")).unwrap();
    assert_eq!(ps.structural_node_pos(slot_kind::SPLIT), Some(5), "fixture split pos");
    assert_eq!(ps.structural_node_pos(slot_kind::MIXER), Some(7), "fixture mixer pos");
    let (blocks_before, grid_before) = (ps.blocks(), ps.grid());

    assert!(ps.set_node_pos(slot_kind::SPLIT, 4), "split node found and mutated");
    assert!(ps.set_node_pos(slot_kind::MIXER, 8), "mixer node found and mutated");

    assert_eq!(ps.structural_node_pos(slot_kind::SPLIT), Some(4), "split pos written");
    assert_eq!(ps.structural_node_pos(slot_kind::MIXER), Some(8), "mixer pos written");
    assert!(ps.is_split(), "split flag untouched");
    assert_eq!(ps.blocks(), blocks_before, "blocks untouched");
    assert_eq!(ps.grid(), grid_before, "grid cells untouched (columns are array-based)");

    // The blob carries the mutated preset map verbatim (`to_blob` fidelity is pinned by
    // `to_blob_round_trips_the_preset`; here we just confirm the mutation is inside it).
    let (seq, _) = fretwire_data::stream::read_sequence(&ps.to_blob(), 3);
    assert_eq!(seq.len(), 3, "blob holds magic + header + preset");
    let g0 = fretwire_data::stream::map_get(&seq[2], 0).unwrap();
    let slots = match fretwire_data::stream::map_get(g0, 22).unwrap() {
        rmpv::Value::Array(a) => a,
        _ => panic!("slot array"),
    };
    let pos_of = |kind: i64| {
        let s = slots
            .iter()
            .find(|s| fretwire_data::stream::map_get(s, 19).and_then(rmpv::Value::as_i64) == Some(kind))
            .unwrap();
        let content = fretwire_data::stream::map_get(s, 20).unwrap();
        let holder = match content {
            rmpv::Value::Map(m) => m
                .iter()
                .map(|(_, v)| v)
                .find(|v| fretwire_data::stream::map_get(v, 8).is_some())
                .unwrap(),
            _ => panic!("content map"),
        };
        fretwire_data::stream::map_get(holder, 13).and_then(rmpv::Value::as_i64)
    };
    assert_eq!(pos_of(slot_kind::SPLIT), Some(4), "blob carries the new split pos");
    assert_eq!(pos_of(slot_kind::MIXER), Some(8), "blob carries the new mixer pos");
}
