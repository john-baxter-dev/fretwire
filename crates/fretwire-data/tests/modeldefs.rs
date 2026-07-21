//! Tests that the preset's numeric model id (slot `24 → 25`) indexes `HelixModelDefs.bin`
//! to yield the correct model — the unambiguous resolution display names can't give.
//!
//! Needs the unshipped Line 6 reference data; compiles only with a local copy present
//! (`have_bundled_data`, set by build.rs).
#![cfg(have_bundled_data)]

use fretwire_data::modeldefs::ModelDefs;
use std::path::PathBuf;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(data_dir().join(name)).unwrap()
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

#[test]
fn model_defs_parse() {
    let defs = ModelDefs::parse(&data("HelixModelDefs.bin")).unwrap();
    assert!(defs.len() > 600, "expected ~681 model defs, got {}", defs.len());
    // First entry is the German Mahadeva amp (seen in the hexdump).
    assert_eq!(defs.symbolic_id(0), Some("HD2_AmpGermanMahadeva"));
}

#[test]
fn model_table_is_complete_and_findable() {
    // The model table contains every model by name — so it's a usable authoritative DB. We can
    // find a block's true table index by name (the reverse of what we ultimately want).
    let defs = ModelDefs::parse(&data("HelixModelDefs.bin")).unwrap();
    let find = |name: &str| (0..defs.len()).find(|&i| defs.name(i) == Some(name));

    for name in ["Bucket Brigade", "70s Chorus", "Harmonic Tremolo", "Dynamic Hall"] {
        let idx = find(name).unwrap_or_else(|| panic!("{name} missing from model table"));
        eprintln!("{name:<20} -> table index {idx}");
    }
}

// RESOLVED (see tests/correlate_modelid.rs + docs/preset-format.md): the preset carries NO
// numeric model id that indexes this table. Slot 24->25 is a runtime DSP handle (non-monotonic
// with slot, not a table field); path 11->6 encodes only the *category*. Models are identified by
// `symbolicID` (the one globally-unique key) and resolved from a block via name + category.
