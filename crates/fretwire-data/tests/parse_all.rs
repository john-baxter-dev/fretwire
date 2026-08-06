//! Validates the data-layer structs against the real files HX Edit ships.
//! These live in `crates/fretwire-data/data/` (copied verbatim from the install `res/` folder).
//!
//! That data is not shipped in the repo, so this file compiles only when a
//! local copy is present (`have_bundled_data`, set by build.rs). A clean checkout skips it.
#![cfg(have_bundled_data)]

use std::path::PathBuf;

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

fn read(name: &str) -> Vec<u8> {
    std::fs::read(data_dir().join(name)).unwrap_or_else(|e| panic!("read {name}: {e}"))
}

#[test]
fn parses_every_models_file() {
    let mut count = 0;
    for entry in std::fs::read_dir(data_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("models") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let file = fretwire_data::ModelFile::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("parse {name}: {e}"));
        assert!(!file.models.is_empty(), "{name} had no models");
        count += 1;
    }
    assert!(count > 0, "no .models files found in {:?}", data_dir());
    eprintln!("parsed {count} .models files");
}

#[test]
fn parses_hx_stomp_default_preset() {
    let p = fretwire_data::Preset::from_slice(&read("default_preset_hxs.hlx")).unwrap();
    assert_eq!(p.version, 6);
    assert_eq!(p.name(), "New Preset");
    // HX Stomp device id observed as 0x210006.
    assert_eq!(p.data.device, 2162694);
}

#[test]
fn preset_round_trips_losslessly() {
    // Parse → serialize → parse and require the JSON value to be identical, proving we
    // drop nothing in the dynamic `tone` tree (needed for sending edits back to the unit).
    let original: serde_json::Value = serde_json::from_slice(&read("default_preset.hlx")).unwrap();
    let preset: fretwire_data::Preset = serde_json::from_value(original.clone()).unwrap();
    let reserialized = serde_json::to_value(&preset).unwrap();
    assert_eq!(
        original, reserialized,
        "preset did not round-trip losslessly"
    );
}
