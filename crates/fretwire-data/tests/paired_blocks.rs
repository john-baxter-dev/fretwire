//! Paired-cab and block-class encoding, checked against the device's own bytes.
//!
//! The oracles are the nine streams of `captures/pairing_sweep.md` — each is the HX Stomp's edit
//! buffer after our own verified `swap` put a known model combination in slot 1, so every file is
//! the device's answer to "what does this combination store". The tests hand-write the `tone` JSON
//! HX Edit would produce for the same block (factory-default values, transcribed from the sweep)
//! and require [`apply_tone`] to reproduce the device's slot byte-for-byte.
//!
//! The fixtures are tracked; the reference data (`Helix.sym`) is Line 6's and is not, so the
//! whole file is data-gated like the other conversion tests.
#![cfg(have_bundled_data)]

use std::path::PathBuf;

use fretwire_data::{
    stream::{PresetStream, map_get},
    symbols::DeviceSymbols,
    tone::apply_tone,
};
use rmpv::Value;
use serde_json::json;

fn fixture(name: &str) -> PresetStream {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../captures")
        .join(name);
    PresetStream::parse(&std::fs::read(&path).unwrap_or_else(|e| panic!("{path:?}: {e}")))
        .expect("fixture parses")
}

fn symbols() -> Option<DeviceSymbols> {
    // Same resolution order as build.rs, which is what set `have_bundled_data`.
    let dir = std::env::var_os("FRETWIRE_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
                .map(|b| b.join("fretwire").join("data"))
        })?;
    DeviceSymbols::parse(&std::fs::read(dir.join("Helix.sym")).ok()?).ok()
}

fn slot_1(ps: &PresetStream) -> Value {
    match map_get(&ps.preset, 0).and_then(|m| map_get(m, 22)) {
        Some(Value::Array(slots)) => slots[1].clone(),
        other => panic!("no slot array: {other:?}"),
    }
}

/// Convert a one-block tone onto the fixture itself and hand back (converted, device) slot 1.
fn converted(name: &str, dsp0: serde_json::Value) -> (Value, Value) {
    let Some(syms) = symbols() else {
        panic!("have_bundled_data is set but Helix.sym is unreadable");
    };
    let oracle = fixture(name);
    let mut donor = oracle.clone();
    let tone = json!({ "dsp0": dsp0 });
    apply_tone(&mut donor, tone.as_object().unwrap(), &syms).expect("conversion");
    (slot_1(&donor), slot_1(&oracle))
}

/// Content key from a slot: `slot → 20 → key`.
fn content(slot: &Value, key: i64) -> Value {
    map_get(map_get(slot, 20).expect("content"), key)
        .cloned()
        .unwrap_or(Value::Nil)
}

/// An amp paired with a **legacy** cab. Class 18, and the cab bank is the legacy layout: the
/// symbol's five parameters then `@mic`, counts 6 stored / 5 from the symbol.
///
/// The cab's high cut is deliberately spelled `"High Cut"` — the older HX Edit spelling, 26 of
/// 168 entries in the real backup — so the spelling-drift lookup is exercised against real bytes.
#[test]
fn amp_with_legacy_cab_matches_the_device() {
    let (ours, device) = converted(
        "pairing_amp_legacy_cab.msgpack.bin",
        json!({
            "block0": {
                "@model": "HD2_AmpUSDoubleNrm", "@type": 3, "@enabled": true,
                "@path": 0, "@position": 0, "@cab": "cab0",
                "Drive": 0.35, "Bass": 0.44, "Mid": 0.52, "Treble": 0.57, "Presence": 0.1,
                "ChVol": 0.85, "Master": 1.0, "Sag": 0.5, "Hum": 0.5, "Ripple": 0.5,
                "Bias": 0.6, "BiasX": 0.5,
            },
            "cab0": {
                "@model": "HD2_Cab1x12USDeluxe", "@enabled": true, "@mic": 6,
                "Distance": 2.0, "LowCut": 80.0, "High Cut": 8000.0,
                "EarlyReflections": 0.0, "Level": 0.0,
            },
        }),
    );
    assert_eq!(ours, device);
}

/// An amp paired with a **new-family** cab. Class 33, and the cab bank is the `CabMicIr` layout:
/// `Mic` first as an integer, the trailing `IrData` not stored (7 of the symbol's 8).
#[test]
fn amp_with_micir_cab_matches_the_device() {
    let (ours, device) = converted(
        "pairing_amp_micir_cab.msgpack.bin",
        json!({
            "block0": {
                "@model": "HD2_AmpUSDoubleNrm", "@type": 3, "@enabled": true,
                "@path": 0, "@position": 0, "@cab": "cab0",
                "Drive": 0.35, "Bass": 0.44, "Mid": 0.52, "Treble": 0.57, "Presence": 0.1,
                "ChVol": 0.85, "Master": 1.0, "Sag": 0.5, "Hum": 0.5, "Ripple": 0.5,
                "Bias": 0.6, "BiasX": 0.5,
            },
            "cab0": {
                "@model": "HD2_CabMicIr_1x12USDeluxe", "@enabled": true,
                "Mic": 0, "Position": 0.19, "Distance": 1.0, "Angle": 45.0,
                "LowCut": 19.9, "HighCut": 20100.0, "Level": 0.0,
            },
        }),
    );
    assert_eq!(ours, device);
}

/// A dual **legacy** cab (`@type` 4): the main block is itself a cab, the sibling the second.
/// Class 16, both banks in the legacy layout.
#[test]
fn dual_legacy_cab_matches_the_device() {
    let (ours, device) = converted(
        "pairing_dual_legacy_cab.msgpack.bin",
        json!({
            "block0": {
                "@model": "HD2_Cab1x12USDeluxe", "@type": 4, "@enabled": true,
                "@path": 0, "@position": 0, "@cab": "cab0", "@mic": 6,
                "Distance": 2.0, "LowCut": 80.0, "HighCut": 8000.0,
                "EarlyReflections": 0.0, "Level": 0.0,
            },
            "cab0": {
                "@model": "HD2_Cab1x15TucknGo", "@enabled": true, "@mic": 10,
                "Distance": 1.0, "LowCut": 20.0, "HighCut": 8000.0,
                "EarlyReflections": 0.0, "Level": 0.0,
            },
        }),
    );
    assert_eq!(ours, device);
}

/// A dual **new-family** cab: two `WithPan` symbols, class 32, 9 of 10 parameters stored each.
#[test]
fn dual_micir_cab_matches_the_device() {
    let (ours, device) = converted(
        "pairing_dual_micir_cab.msgpack.bin",
        json!({
            "block0": {
                "@model": "HD2_CabMicIr_4x12CaliV30WithPan", "@type": 4, "@enabled": true,
                "@path": 0, "@position": 0, "@cab": "cab0",
                "Mic": 10, "Position": 0.23, "Distance": 1.0, "Angle": 0.0,
                "LowCut": 19.9, "HighCut": 20100.0, "Level": 0.0, "Pan": 0.5, "Delay": 0.0,
            },
            "cab0": {
                "@model": "HD2_CabMicIr_2x12JazzRivetWithPan", "@enabled": true,
                "Mic": 0, "Position": 0.23, "Distance": 1.0, "Angle": 0.0,
                "LowCut": 19.9, "HighCut": 20100.0, "Level": 0.0, "Pan": 0.5, "Delay": 0.0,
            },
        }),
    );
    assert_eq!(ours, device);
}

/// An IR block, whole: no second bank; in its place content key `27` carries the referenced IR's
/// UUID, NUL-terminated — the tone's `@uuid`. The base model name also proves the suffix
/// resolution: the tone writes `HD2_ImpulseResponse1024`, the device symbol is the `Mono` form.
#[test]
fn an_ir_block_matches_the_device() {
    let (ours, device) = converted(
        "class_ir_mono.msgpack.bin",
        json!({ "block0": {
            "@model": "HD2_ImpulseResponse1024", "@type": 5, "@enabled": true,
            "@path": 0, "@position": 0, "@uuid": "4b41c57b04c05b1471277ecf74231a7d",
            "Index": 1, "LowCut": 19.9, "HighCut": 20100.0, "Mix": 1.0, "Level": -18.0,
        }}),
    );
    assert_eq!(ours, device);
}

/// An IR block aimed at an **empty** IR slot — the device stores key `27` as the empty string,
/// which is also what a tone with no `@uuid` means.
#[test]
fn an_unreferenced_ir_block_matches_the_device() {
    let (ours, device) = converted(
        "class_ir_unreferenced.msgpack.bin",
        json!({ "block0": {
            "@model": "HD2_ImpulseResponse1024", "@type": 5, "@enabled": true,
            "@path": 0, "@position": 0,
            "Index": 2, "LowCut": 19.9, "HighCut": 20100.0, "Mix": 1.0, "Level": -18.0,
        }}),
    );
    assert_eq!(ours, device);
}

/// The remaining sweep classes: model resolution and the class/count trio, without transcribing
/// every default value (the value path itself is pinned by the byte-for-byte tests above).
#[test]
fn standalone_classes_match_the_device() {
    // (fixture, tone block, expected model index)
    let cases: [(&str, serde_json::Value, i64); 4] = [
        (
            "class_legacy_cab_standalone.msgpack.bin",
            json!({ "@model": "HD2_Cab1x12USDeluxe", "@type": 2, "@path": 0, "@position": 0, "@mic": 6 }),
            49,
        ),
        (
            "class_micir_cab_standalone.msgpack.bin",
            json!({ "@model": "HD2_CabMicIr_1x12USDeluxe", "@type": 2, "@path": 0, "@position": 0 }),
            709,
        ),
        // A dual IR concatenates its two slot UUIDs into the one key-27 string.
        (
            "class_ir_dual.msgpack.bin",
            json!({ "@model": "HD2_ImpulseResponse1024Dual", "@type": 5, "@path": 0, "@position": 0,
                    "@uuid": "4b41c57b04c05b1471277ecf74231a7d4b41c57b04c05b1471277ecf74231a7d" }),
            708,
        ),
        (
            "class_synth.msgpack.bin",
            json!({ "@model": "HD2_Synth3NoteGenerator", "@type": 8, "@stereo": false, "@path": 0, "@position": 0 }),
            377,
        ),
    ];
    for (name, block, index) in cases {
        let (ours, device) = converted(name, json!({ "block0": block }));
        assert_eq!(content(&ours, 9), content(&device, 9), "{name}: class");
        assert_eq!(
            content(&ours, 24),
            content(&device, 24),
            "{name}: model ref"
        );
        assert_eq!(
            map_get(&content(&device, 24), 25).and_then(Value::as_i64),
            Some(index),
            "{name}: fixture holds the expected model"
        );
        // The second bank must match in shape too: absent on an IR block (key 27 instead),
        // an empty bank everywhere else unpaired.
        assert_eq!(content(&ours, 27), content(&device, 27), "{name}: key 27");
        for bank in [11, 12] {
            for count in [2, 3] {
                assert_eq!(
                    map_get(&content(&ours, bank), count).cloned(),
                    map_get(&content(&device, bank), count).cloned(),
                    "{name}: bank {bank} count {count}"
                );
            }
        }
    }
}

/// A **Looper** block, checked against a device-written one: the "Sultans" Floor stream holds a
/// looper at slot 8 and the same unit's `.hxb` holds its tone (`FACTORY 2` slot 64). Slot kind 7,
/// class 22, model index at key `8`, and only the tone's four preset parameters stored — not the
/// symbol's ten.
///
/// These inputs are a contributor's own preset data and are not in git, so this skips on a clean
/// clone, like the `tone_to_wire` oracle they also serve. (The capture recorded a model being
/// swapped in another block, so only the looper's slot is compared.)
#[test]
fn a_looper_matches_the_device() {
    let floor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/helix-floor");
    let Some(syms) = symbols() else { return };
    let Some(oracle) = std::fs::read(floor.join("floor-sultans.msgpack.bin"))
        .ok()
        .and_then(|b| PresetStream::parse(&b).ok())
    else {
        return;
    };
    let Some(tone) = std::fs::read_dir(&floor)
        .ok()
        .and_then(|d| {
            d.flatten()
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "hxb"))
        })
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| fretwire_data::hxb::Hxb::parse(&b).ok())
        .and_then(|hxb| hxb.setlists().get(1)?.presets.get(64)?.clone())
        .map(|p| p.tone)
    else {
        return;
    };
    let mut donor = oracle.clone();
    apply_tone(&mut donor, tone.as_object().unwrap(), &syms).expect("conversion");
    let slot = |ps: &PresetStream| match map_get(&ps.preset, 0).and_then(|m| map_get(m, 22)) {
        Some(Value::Array(slots)) => slots[8].clone(),
        other => panic!("no slot array: {other:?}"),
    };
    assert_eq!(slot(&donor), slot(&oracle));
}

/// Mixing the families in one dual block is a shape the device has never shown us — refused.
#[test]
fn a_mixed_family_dual_cab_is_refused() {
    let Some(syms) = symbols() else {
        panic!("have_bundled_data is set but Helix.sym is unreadable");
    };
    let mut donor = fixture("pairing_dual_legacy_cab.msgpack.bin");
    let tone = json!({ "dsp0": {
        "block0": {
            "@model": "HD2_Cab1x12USDeluxe", "@type": 4, "@path": 0, "@position": 0,
            "@cab": "cab0", "@mic": 6,
        },
        "cab0": { "@model": "HD2_CabMicIr_1x12USDeluxe" },
    }});
    let err = apply_tone(&mut donor, tone.as_object().unwrap(), &syms).unwrap_err();
    assert!(
        err.to_string().contains("mixes the families"),
        "wrong refusal: {err}"
    );
}
