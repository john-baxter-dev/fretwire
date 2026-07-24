//! End-to-end: a captured device preset stream + the reference data -> a typed, named, editable
//! model, and a byte-exact bypass edit command. Exercises every layer together.
//!
//! Needs the (unshipped) Line 6 reference data, so the whole file compiles only when a local copy
//! is present (`have_bundled_data`, set by build.rs).
#![cfg(have_bundled_data)]

use fretwire_core::editor::Catalog;
use fretwire_data::stream::ParamValue;
use std::path::PathBuf;

// Capture fixtures (our own device recordings) live in the repo under captures/.
fn data(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures").join(name);
    std::fs::read(p).unwrap()
}

// The catalog loads the Line 6 reference data from the `fretwire import-data` cache.
fn catalog() -> Catalog {
    Catalog::from_data_dir(&fretwire_core::data_dir()).unwrap()
}

#[test]
fn loads_preset_into_named_editable_blocks() {
    let cat = catalog();
    let preset = cat.load_preset(&data("preset1_stream.msgpack.bin")).unwrap();

    assert_eq!(preset.device_model.as_deref(), Some("P33"));
    assert!(preset.firmware.unwrap().starts_with("v3.71"));
    // Six blocks enumerated from the slot array — the signal path lists only four; the amp+cab
    // and a second reverb live off-path and were previously dropped.
    assert_eq!(preset.blocks.len(), 6);

    // DSP meter: this 6-block preset (incl. an amp) draws a plausible, non-trivial slice of the
    // budget, and the amp+cab block is the single biggest consumer.
    assert!(preset.dsp_load > 10.0 && preset.dsp_load < 100.0, "dsp_load = {}", preset.dsp_load);
    let amp = preset.blocks.iter().find(|b| b.paired_model_name.is_some()).unwrap();
    assert!(amp.dsp_load.unwrap() > 20.0, "amp+cab load = {:?}", amp.dsp_load);

    // The Harmonic Tremolo block: resolved id, mono variant, named params, current values.
    let ht = preset.blocks.iter().find(|b| b.model_name == "Harmonic Tremolo").unwrap();
    assert_eq!(ht.slot, 4);
    assert_eq!(ht.user_label.as_deref(), Some("Tremolo"));
    assert_eq!(ht.symbolic_id.as_deref(), Some("HD2_TremoloHarmonic"));
    assert_eq!(ht.variant, Some("Mono")); // 10 values -> Mono device order

    // Params are named in device order and carry values; index 8 is SyncSelect1 (=6), not Spread.
    let by_name = |n: &str| ht.params.iter().find(|p| p.name == n).map(|p| p.value);
    assert_eq!(by_name("Mix"), Some(ParamValue::Float(1.0)));
    assert_eq!(by_name("BassFreq"), Some(ParamValue::Float(500.0)));
    assert_eq!(ht.params[8].name, "SyncSelect1");
    assert!(by_name("Spread").is_none());

    // Params carry UI metadata (range + widget) from the model table, matched by name.
    let mix = ht.params.iter().find(|p| p.name == "Mix").unwrap();
    assert_eq!(mix.meta.min, Some(0.0));
    assert_eq!(mix.meta.max, Some(1.0));
    assert_eq!(mix.meta.display_type.as_deref(), Some("percent"));
    let bass = ht.params.iter().find(|p| p.name == "BassFreq").unwrap();
    assert_eq!((bass.meta.min, bass.meta.max), (Some(40.0), Some(2000.0)));
    assert_eq!(bass.meta.display_type.as_deref(), Some("frequency"));
}

#[test]
fn lists_swap_candidates_in_a_block_category() {
    let cat = catalog();
    let preset = cat.load_preset(&data("preset1_stream.msgpack.bin")).unwrap();
    let ht = preset.blocks.iter().find(|b| b.model_name == "Harmonic Tremolo").unwrap();
    let category = ht.category.expect("tremolo has a category");

    let choices = cat.models_in_category(category, ht.variant);
    assert!(choices.len() > 3, "expected several models in the category, got {}", choices.len());

    // The current model is itself a candidate, and every candidate is in the same category and
    // resolves back to a real Helix.sym index.
    assert!(choices.iter().any(|c| c.symbolic_id == "HD2_TremoloHarmonic"));
    assert!(choices.iter().all(|c| c.category == Some(category)));
    assert!(choices.iter().all(|c| cat.symbols.by_index(c.index as usize).is_some()));
    // No duplicate models (one entry per symbolic id).
    let mut ids: Vec<&str> = choices.iter().map(|c| c.symbolic_id.as_str()).collect();
    ids.sort();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "candidates should be de-duplicated per model");

    // Deterministic order (the reported "DSP % shuffling" bug was HashMap iteration nondeterminism).
    assert_eq!(choices, cat.models_in_category(category, ht.variant), "ordering must be stable");

    // After disambiguation, display names are unique within the list (no two identical rows).
    let mut names: Vec<&str> = choices.iter().map(|c| c.name.as_str()).collect();
    names.sort();
    let m = names.len();
    names.dedup();
    assert_eq!(names.len(), m, "display names should be disambiguated to be unique");
}

#[test]
fn lists_named_categories_for_the_selector() {
    let cat = catalog();
    let cats = cat.models_in_category(1, None);
    assert!(!cats.is_empty());
    let categories = cat.categories();
    // Includes the staple effect types, each with a name; ids resolve via category_name.
    let names: Vec<&str> = categories.iter().map(|(_, n)| *n).collect();
    for want in ["Amp", "Delay", "Reverb", "Distortion", "Modulation"] {
        assert!(names.contains(&want), "categories should include {want}: {names:?}");
    }
    assert!(categories.iter().all(|(id, n)| fretwire_core::editor::category_name(*id) == Some(*n)));
}

#[test]
fn collision_disambiguation_amp_vs_preamp() {
    // The amp catalog (category 1) contains both an amp and a preamp "EV Panama Red"/"Blue" — same
    // display name, distinct models. They must both appear, with type tokens appended.
    let cat = catalog();
    let amp_cat = 1;
    let choices = cat.models_in_category(amp_cat, None);
    let panamas: Vec<&str> =
        choices.iter().filter(|c| c.name.starts_with("EV Panama Red")).map(|c| c.name.as_str()).collect();
    assert!(
        panamas.contains(&"EV Panama Red (Amp)") && panamas.contains(&"EV Panama Red (Preamp)"),
        "expected disambiguated amp+preamp entries, got {panamas:?}"
    );
}

#[test]
fn block_produces_byte_exact_bypass_edit() {
    let cat = catalog();
    let preset = cat.load_preset(&data("preset1_stream.msgpack.bin")).unwrap();
    let ht = preset.blocks.iter().find(|b| b.model_name == "Harmonic Tremolo").unwrap();

    // The editor model emits the exact wire bytes captured from HX Edit (slot 4, enabled, txn 0x03f2).
    let body = ht.set_enabled_edit(true, 0x03f2);
    assert_eq!(
        body,
        vec![0x83, 0x66, 0xcd, 0x03, 0xf2, 0x64, 0x29, 0x65, 0x82, 0x62, 0x04, 0x3b, 0xc3]
    );

    // And it parses back to a bypass of this block's slot.
    let decoded = fretwire_protocol::EditBody::parse(&body).unwrap();
    assert_eq!(decoded.slot, Some(ht.slot));
    assert_eq!(decoded.value, fretwire_protocol::EditValue::Bool(true));
}

#[test]
fn block_produces_param_set_edit_by_name() {
    // Parameter editing is computable: name a param, get the byte-exact set-value command.
    let cat = catalog();
    let preset = cat.load_preset(&data("preset1_stream.msgpack.bin")).unwrap();
    let ht = preset.blocks.iter().find(|b| b.model_name == "Harmonic Tremolo").unwrap();

    // Mix is index 7 in this block's device order; set it to 0.5.
    let mix = ht.params.iter().find(|p| p.name == "Mix").unwrap();
    assert_eq!(mix.index, 7);
    let body = ht.set_param_by_name("Mix", 0.5, 0x0100).unwrap();
    let decoded = fretwire_protocol::EditBody::parse(&body).unwrap();
    assert_eq!(decoded.op, fretwire_protocol::edit::OP_SET_VALUE);
    assert_eq!(decoded.slot, Some(4));
    assert_eq!(decoded.param_index, Some(7));
    assert_eq!(decoded.value, fretwire_protocol::EditValue::Float(0.5));
}

#[test]
fn reverb_params_named_with_trails() {
    // Dynamic Hall = symbolicID VIC_ReverbRotating. Its 13 values = the 12-param symbol + the
    // trailing Trails switch; resolve_order matches symbol+1 and names every param.
    let cat = catalog();
    let preset = cat.load_preset(&data("preset1_stream.msgpack.bin")).unwrap();
    let dh = preset.blocks.iter().find(|b| b.model_name == "Dynamic Hall").unwrap();

    assert_eq!(dh.symbolic_id.as_deref(), Some("VIC_ReverbRotating"));
    // The device's own model index (610) says Stereo — our old count-heuristic guessed Mono
    // (both variants declare 12 params, so length couldn't disambiguate them).
    assert_eq!(dh.variant, Some("Stereo"));
    assert_eq!(dh.params.len(), 13);
    assert_eq!(dh.params[0].name, "Decay");
    assert_eq!(dh.params[12].name, "Trails");
}

#[test]
fn split_preset_exposes_editable_routing_nodes() {
    let cat = catalog();
    let preset = cat.load_preset(&data("split_preset_stream.msgpack.bin")).unwrap();
    assert!(preset.split(), "fixture is a split preset");

    // Split node resolves to a known type (Split Y) and is selectable/matchable via SPLIT_TYPES.
    let split = preset.split_node().expect("split node present");
    assert_eq!(split.symbolic_id.as_deref(), Some("HD2_AppDSPFlowSplitY"));
    assert!(preset.is_split_node(split.slot));
    assert!(fretwire_core::editor::SPLIT_TYPES.iter().any(|(_, s, _)| *s == "HD2_AppDSPFlowSplitY"));

    // Mixer node resolves to the join model, with named, *editable* A/B params (ranges injected).
    let mixer = preset.mixer_node().expect("mixer node present");
    assert!(preset.is_mixer_node(mixer.slot));
    let a_level = mixer.params.iter().find(|p| p.name == "A Level").expect("mixer has A Level");
    assert!(a_level.meta.min.is_some() && a_level.meta.max.is_some(), "A Level is an editable slider");
    let a_pan = mixer.params.iter().find(|p| p.name == "A Pan").expect("mixer has A Pan");
    assert_eq!((a_pan.meta.min, a_pan.meta.max), (Some(0.0), Some(1.0)));

    // The routing nodes are addressable through the node-aware lookup used by the param handlers.
    assert!(preset.block(split.slot).is_some());
    assert!(preset.block(mixer.slot).is_some());
}

#[test]
fn split_preset_classifies_common_vs_path_a_vs_path_b() {
    // "Dual Amp" (live dump): Tremolo(slot4) common-before, US Princess(slot6) path A,
    // Reverb(slot7) common-after, GSG100(slot15) path B; split_pos=5, mixer_pos=7.
    let cat = catalog();
    let p = cat.load_preset(&data("dual_amp_stream.msgpack.bin")).unwrap();
    assert!(p.split());
    assert_eq!(p.split_pos(), Some(5));
    assert_eq!(p.mixer_pos(), Some(7));

    let by_name = |n: &str| p.blocks.iter().find(|b| b.model_name.contains(n)).expect(n);
    let trem = by_name("Tremolo");
    let usp = by_name("Princess");
    let gsg = by_name("GSG");

    // Path B = the bottom row.
    assert_eq!(gsg.row, 1, "GSG is on path B (bottom row)");
    // Top row holds common + path A; classify by slot vs split_pos/mixer_pos.
    assert_eq!(trem.row, 0);
    assert!(trem.slot < p.split_pos().unwrap(), "Tremolo is common (pre-split)");
    let (sp, mp) = (p.split_pos().unwrap(), p.mixer_pos().unwrap());
    assert!(usp.slot >= sp && usp.slot < mp, "US Princess is on path A");
}

// The input/output nodes resolve to named, ranged, editable params: the io.models meta (now in the
// bundled param table) gives ranges/widgets, the Helix.sym order gives wire indexes, and the code-ish
// sym names get display names ("noiseGate" → "Input Gate").
#[test]
fn io_nodes_resolve_named_params() {
    let p = catalog().load_preset(&data("preset1_stream.msgpack.bin")).unwrap();

    let input = p.input_node().expect("input node");
    assert_eq!(input.slot, 0);
    assert_eq!(input.model_name, "Input");
    let names: Vec<&str> = input.params.iter().map(|q| q.name.as_str()).collect();
    assert_eq!(names, ["Input Gate", "Threshold", "Decay"]);
    assert_eq!(input.params[0].meta.value_type, Some(2), "gate is a bool switch");
    assert_eq!(input.params[1].meta.min, Some(-96.0), "threshold range from io.models");
    assert_eq!(input.params[1].meta.max, Some(0.0));

    let output = p.output_node().expect("output node");
    assert_eq!(output.slot, 9);
    assert_eq!(output.model_name, "Output");
    let names: Vec<&str> = output.params.iter().map(|q| q.name.as_str()).collect();
    assert_eq!(names, ["Pan", "Level"]);
    assert_eq!(output.params[1].meta.min, Some(-120.0), "level range from io.models");
    assert_eq!(output.params[1].meta.max, Some(20.0));

    // Wire addressing: block() finds them by slot (history labels, param edits).
    assert_eq!(p.block(0).map(|b| b.model_name.as_str()), Some("Input"));
    assert_eq!(p.block(9).map(|b| b.model_name.as_str()), Some("Output"));
}
