//! Model-id correlation: how a preset block maps to its `HelixModelDefs.bin` entry.
//!
//! These assert the findings from the investigation (see `docs/preset-format.md`):
//!   1. `symbolicID` is the only globally-unique key (681/681); display names collide (163).
//!   2. The preset's `11 → 6` encodes *category*, not model identity — two different models in
//!      the same category share it (Harmonic Tremolo & 70s Chorus are both 1037 / category 8).
//!   3. The preset's `24 → 25` is the block's **model identity** — an index into `Helix.sym`'s
//!      array order (NOT a field in `HelixModelDefs.bin`, which is why it first looked like an
//!      opaque handle). Strip the resolved symbol's `Mono`/`Stereo` suffix to get the `symbolicID`.
//!   4. (name, category, param-count) is unique for every real model except the `Match H30/G25`
//!      cab pair (a duplicate-name defect in Line 6's data) and per-device I/O blocks.
//!
//! Run the descriptive dumps with: `cargo test -p fretwire-data --test correlate_modelid -- --nocapture`
//!
//! Needs the unshipped Line 6 reference data; compiles only with a local copy present
//! (`have_bundled_data`, set by build.rs).
#![cfg(have_bundled_data)]

use fretwire_data::modeldefs::ModelDefs;
use fretwire_data::stream::PresetStream;
use std::collections::HashMap;
use std::path::PathBuf;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(data_dir().join(name)).unwrap()
}

// Our own captured preset stream lives in the in-repo `captures/` dir (not Line 6 data, so it's
// not part of the import cache).
fn capture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../captures")
        .join(name);
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

fn defs() -> ModelDefs {
    ModelDefs::parse(&data("HelixModelDefs.bin")).unwrap()
}

fn preset() -> PresetStream {
    PresetStream::parse(&capture("preset1_stream.msgpack.bin")).unwrap()
}

#[test]
fn symbolic_id_is_the_only_unique_key() {
    let d = defs();
    let mut names: HashMap<&str, u32> = HashMap::new();
    let mut syms: HashMap<&str, u32> = HashMap::new();
    for i in 0..d.len() {
        if let Some(n) = d.name(i) {
            *names.entry(n).or_default() += 1;
        }
        if let Some(s) = d.symbolic_id(i) {
            *syms.entry(s).or_default() += 1;
        }
    }
    // symbolicID: globally unique. name: many collisions (cab/amp/preamp/legacy variants).
    assert_eq!(
        syms.len(),
        d.len(),
        "symbolicID must be unique across the whole table"
    );
    let colliding_names = names.values().filter(|&&c| c > 1).count();
    assert!(
        colliding_names > 100,
        "expected many colliding display names, got {colliding_names}"
    );
}

#[test]
fn path_key_11_6_is_category_not_model_identity() {
    // Harmonic Tremolo and 70s Chorus are DIFFERENT models (different symbolicID & param count)
    // yet share path `11 → 6` = 1037 — both are category 8. So 11→6 cannot identify a model.
    let ps = preset();
    let by_name: HashMap<String, i64> = ps
        .footswitch_layout()
        .into_iter()
        .flatten()
        .filter_map(|p| p.model_id.map(|id| (p.model_name, id)))
        .collect();
    assert_eq!(by_name.get("Harmonic Tremolo"), Some(&1037));
    assert_eq!(by_name.get("70s Chorus"), Some(&1037));
    assert_ne!(by_name.get("Bucket Brigade"), by_name.get("Dynamic Hall"));

    let d = defs();
    let cat = |name: &str| d.category(d.ids_by_name(name)[0]);
    assert_eq!(cat("Harmonic Tremolo"), cat("70s Chorus")); // same category 8 -> same 11→6
}

#[test]
fn slot_key_24_25_indexes_helix_sym() {
    // CORRECTED finding: `24 → 25` is the block's model identity — an index into `Helix.sym`'s
    // array order. (It looked like an opaque handle only because we'd tested it against the
    // wrong table; it doesn't index `HelixModelDefs.bin`.) Each block's index resolves to the
    // exact device symbol — including the Mono/Stereo variant the count-heuristic couldn't tell.
    use fretwire_data::symbols::DeviceSymbols;
    let syms = DeviceSymbols::parse(&data("Helix.sym")).unwrap();
    let by_slot: HashMap<usize, i64> = preset()
        .effect_blocks()
        .into_iter()
        .filter_map(|b| b.model_ref.map(|r| (b.index, r)))
        .collect();
    let expect = [
        (2, "HD2_DelayBucketBrigadeMono"), // not Stereo — the index settles it
        (3, "HD2_Chorus70sChorusMono"),
        (4, "HD2_TremoloHarmonicMono"),
        (5, "HD2_AmpUSPrincess"), // an off-path amp+cab block the signal path never listed
        (7, "VIC_ReverbRotatingStereo"),
    ];
    for (slot, want) in expect {
        let idx = by_slot[&slot] as usize;
        let (sym, _params) = syms
            .by_index(idx)
            .unwrap_or_else(|| panic!("slot {slot} idx {idx}"));
        assert_eq!(sym, want, "slot {slot}");
    }
}

#[test]
fn name_plus_category_plus_paramcount_is_effectively_unique() {
    let d = defs();
    let mut keys: HashMap<(String, Option<i64>, Option<usize>), u32> = HashMap::new();
    for i in 0..d.len() {
        let k = (
            d.name(i).unwrap_or("?").to_string(),
            d.category(i),
            d.param_count(i),
        );
        *keys.entry(k).or_default() += 1;
    }
    let collisions = keys.values().filter(|&&c| c > 1).count();
    // 4 residual groups: Input, Output (per-device), and the Match H30/G25 cab defect (x2 banks).
    assert!(
        collisions <= 4,
        "expected <=4 residual collisions, got {collisions}"
    );
}

#[test]
fn resolve_effect_blocks_to_symbolic_ids() {
    // The test preset's blocks all have unique names -> resolve cleanly with name alone, and the
    // resolved symbolicIDs are the canonical ids for those models.
    let d = defs();
    let expect = [
        ("Bucket Brigade", "HD2_DelayBucketBrigade"),
        ("70s Chorus", "HD2_Chorus70sChorus"),
        ("Harmonic Tremolo", "HD2_TremoloHarmonic"),
        ("Dynamic Hall", "VIC_ReverbRotating"),
    ];
    for (name, sym) in expect {
        let id = d
            .resolve(name, None)
            .unwrap_or_else(|c| panic!("{name} ambiguous: {c:?}"));
        assert_eq!(d.symbolic_id(id), Some(sym));
    }
}

#[test]
fn resolve_needs_category_for_amp_vs_preamp() {
    // A name shared by an amp and its preamp is ambiguous by name; category disambiguates.
    let d = defs();
    let candidates = match d.resolve("A30 Fawn Brt", None) {
        Err(c) => c,
        Ok(_) => panic!("expected ambiguity for amp/preamp name"),
    };
    assert_eq!(candidates.len(), 2);
    let cats: Vec<_> = candidates.iter().map(|&i| d.category(i)).collect();
    assert_ne!(cats[0], cats[1], "amp and preamp should differ in category");
    // Given the amp's category, resolution becomes unique.
    let amp_cat = d.category(candidates[0]);
    let id = d
        .resolve("A30 Fawn Brt", amp_cat)
        .expect("unique with category");
    assert!(d.symbolic_id(id).unwrap().starts_with("HD2_"));
}

// ---- descriptive dumps (research aids; assert nothing of substance) --------------------------

#[test]
fn dump_preset_block_numbers() {
    let ps = preset();
    eprintln!("=== path blocks: name / 11->6 / slot ===");
    for p in ps.footswitch_layout().into_iter().flatten() {
        eprintln!(
            "  {:<18} 11->6={:?} slot={:?}",
            p.model_name, p.model_id, p.slot
        );
    }
    eprintln!("=== effect blocks: slot / 24->25 (handle) ===");
    for b in ps.effect_blocks() {
        eprintln!("  slot {:<2} 24->25={:?}", b.index, b.model_ref);
    }
}
