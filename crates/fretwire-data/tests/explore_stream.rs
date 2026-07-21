//! Exploration harness for the preset MessagePack stream. Run with output visible:
//!   cargo test -p fretwire-data --test explore_stream -- --nocapture
//! This is a research aid (it prints structure); it asserts only that a root is found.
//!
//! Uses a captured preset fixture from the unshipped fretwire-data/data dir; compiles only with a local
//! copy present (`have_bundled_data`, set by build.rs).
#![cfg(have_bundled_data)]

use std::path::PathBuf;

fn blob() -> Vec<u8> {
    // Our own capture fixture lives in the repo under captures/.
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/preset1_stream.msgpack.bin");
    std::fs::read(p).expect("read preset stream blob")
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
fn locates_and_summarizes_root() {
    let data = blob();
    eprintln!("stream is {} bytes; first 24: {:02x?}", data.len(), &data[..24]);

    let root = fretwire_data::stream::locate_root(&data, 64)
        .expect("should find a MessagePack container root");
    eprintln!(
        "\nMessagePack root at offset {} (consumed {} of {} bytes)\n{}",
        root.offset,
        root.consumed,
        data.len(),
        fretwire_data::stream::summarize(&root.value, 2)
    );

    // Sanity: the root should account for most of the stream.
    assert!(root.consumed > data.len() / 2, "root consumed too little — envelope/offset wrong");

    // The root is an envelope map {102: size, 103: ?, 104: <preset blob>}. Key 104 holds a
    // binary string whose content is *itself* MessagePack — the real preset. Drill in.
    let inner = map_get(&root.value, 104).expect("key 104 present");
    let bytes = value_bytes(inner).expect("key 104 is a string/binary blob");
    eprintln!("\nkey 104 blob is {} bytes; first 16: {:02x?}", bytes.len(), &bytes[..16.min(bytes.len())]);

    // The blob is a flat sequence of concatenated MessagePack values.
    let (seq, consumed) = fretwire_data::stream::read_sequence(bytes, 80);
    eprintln!("\nINNER sequence: {} values, consumed {} of {} bytes", seq.len(), consumed, bytes.len());
    for (i, v) in seq.iter().enumerate() {
        eprintln!("  [{i:>3}] {}", fretwire_data::stream::summarize(v, 1));
    }

    // The last value is the preset map — expand it deeply.
    if let Some(preset) = seq.last() {
        eprintln!("\n===== PRESET MAP (deep) =====\n{}", fretwire_data::stream::summarize(preset, 4));
    }
}

#[test]
fn locate_model_names() {
    // Find where strings like "Bucket Brigade" live in the blob, and dump preset keys 5/6/10.
    use fretwire_data::stream::{summarize, PresetStream};
    let data = blob();
    let needle = b"Bucket Brigade";
    if let Some(pos) = data.windows(needle.len()).position(|w| w == needle) {
        let start = pos.saturating_sub(8);
        eprintln!("'Bucket Brigade' at blob offset {pos}; context bytes: {:02x?}", &data[start..(pos + 24).min(data.len())]);
    } else {
        eprintln!("'Bucket Brigade' not found as raw ASCII");
    }
    let ps = PresetStream::parse(&data).unwrap();
    // key 3 = signal paths; expand deeply to see if the Map{7} path nodes hold model names.
    if let Some(v) = ps.field(3) {
        eprintln!("\n----- preset key 3 (paths) -----\n{}", summarize(v, 6));
    }
}

#[test]
fn loaded_blocks_combine_identity_and_values() {
    use fretwire_data::stream::{ParamValue, PresetStream};

    let ps = PresetStream::parse(&blob()).unwrap();
    let loaded = ps.loaded_blocks();

    // Six populated effect blocks (enumerated from the slot array — the serial path lists only
    // four), each with a model index + a non-empty value vector.
    assert_eq!(loaded.len(), 6);
    for b in &loaded {
        assert!(b.model_index.is_some(), "slot {} had no model index", b.slot);
        assert!(!b.params.is_empty(), "slot {} had no params", b.slot);
    }

    // The renamed Harmonic Tremolo (Helix.sym index 318) carries its label, slot, and values.
    let ht = loaded.iter().find(|b| b.model_index == Some(318)).unwrap();
    assert_eq!(ht.user_label.as_deref(), Some("Tremolo"));
    assert_eq!(ht.slot, 4);
    assert_eq!(ht.params.get(4), Some(&ParamValue::Float(500.0)));
}

#[test]
fn reads_param_values_via_path_to_slot() {
    // End-to-end on device data only (deterministic): preset stream -> path block -> slot ->
    // param value at a known index. Demonstrates we extract real per-block parameter values.
    use fretwire_data::stream::{ParamValue, PresetStream};

    let ps = PresetStream::parse(&blob()).unwrap();
    let blocks = ps.blocks();
    let val = |model_name: &str, idx: usize| -> Option<ParamValue> {
        let pb = ps.footswitch_layout().into_iter().flatten().find(|p| p.model_name == model_name)?;
        let s = pb.slot?;
        let slot = blocks.iter().find(|b| b.index as i64 == s)?;
        slot.params.get(idx).copied()
    };

    // Values verified against model defaults in the zip dump (BassFreq=500, TrebFreq=700, etc.).
    assert_eq!(val("Harmonic Tremolo", 4), Some(ParamValue::Float(500.0)));
    assert_eq!(val("Harmonic Tremolo", 5), Some(ParamValue::Float(700.0)));
    assert_eq!(val("Dynamic Hall", 0), Some(ParamValue::Float(4.0)));
    assert_eq!(val("Bucket Brigade", 0), Some(ParamValue::Float(0.122)));
}

// NOTE: resolving a block's params by NAME via `.models` is ambiguous — display names are not
// unique across the model files (mono/stereo variants etc.). Robust naming needs the model
// id/symbolicID (path key 11->6), decoded later. `load_all_models` keyed by name is fine for
// existence checks but must not be relied on for a *specific* model's param order.

#[test]
fn zip_param_names_to_values() {
    // For each path block, resolve its model in .models, fetch its slot's value vector, and zip
    // param names <-> values to see how the device vector aligns with the model's param list.
    use fretwire_data::stream::{ParamValue, PresetStream};

    let ps = PresetStream::parse(&blob()).unwrap();
    let by_name = load_all_models();
    let blocks = ps.blocks();

    for pb in ps.footswitch_layout().into_iter().flatten() {
        let model = match by_name.get(&pb.model_name) {
            Some(m) => m,
            None => continue,
        };
        let slot = pb.slot.and_then(|s| blocks.iter().find(|b| b.index as i64 == s));
        let values = slot.map(|b| b.params.as_slice()).unwrap_or(&[]);
        eprintln!(
            "\n== {} (slot {:?})  model params={} device values={} ==",
            pb.model_name, pb.slot, model.params.len(), values.len()
        );
        for (i, p) in model.params.iter().enumerate() {
            let v = values.get(i);
            let vs = match v {
                Some(ParamValue::Float(f)) => format!("{f}"),
                Some(ParamValue::Int(n)) => format!("{n}"),
                Some(ParamValue::Bool(b)) => format!("{b}"),
                None => "—".into(),
            };
            eprintln!("   {:<16} = {:<8} (model default {:?}, type {:?})",
                p.symbolic_id, vs, p.default_f64().or_else(|| p.default_bool().map(|b| b as i64 as f64)), p.value_type);
        }
        if values.len() > model.params.len() {
            eprintln!("   (+{} trailing device values beyond model params)", values.len() - model.params.len());
        }
    }
}

#[test]
fn correlate_path_to_slots() {
    // Hypothesis: a path node's key 11->8 is the slot index into the block-slots array.
    use fretwire_data::stream::{map_get, PresetStream};
    use rmpv::Value;

    let ps = PresetStream::parse(&blob()).unwrap();
    let positions = match ps.field(3).and_then(|m| map_get(m, 8)) {
        Some(Value::Array(a)) => a,
        _ => panic!(),
    };
    let blocks = ps.blocks();
    eprintln!("=== path node (name, key8) -> slot[key8] (kind, #params) ===");
    for (i, pos) in positions.iter().enumerate() {
        let node = match pos {
            Value::Array(a) => a.first(),
            _ => None,
        };
        let model = node.and_then(|n| map_get(n, 11));
        let name = model
            .and_then(|m| map_get(m, 5))
            .and_then(fretwire_data::stream::value_bytes)
            .map(|b| String::from_utf8_lossy(b).trim_end_matches('\0').to_string());
        let k8 = model.and_then(|m| map_get(m, 8)).and_then(Value::as_i64);
        match (name, k8) {
            (Some(n), Some(slot)) => {
                let b = blocks.iter().find(|b| b.index as i64 == slot);
                eprintln!(
                    "  path[{i}] {:?} key8={slot} -> slot kind={:?} params={:?}",
                    n,
                    b.map(|b| b.kind),
                    b.map(|b| b.params.len())
                );
            }
            _ => eprintln!("  path[{i}] (empty)"),
        }
    }
    // Also list all populated slots for reference.
    eprintln!("\n=== populated slots ===");
    for b in &blocks {
        if !b.params.is_empty() || b.kind == 6 {
            eprintln!("  slot {} kind={} #params={} model_ref={:?}", b.index, b.kind, b.params.len(), b.model_ref);
        }
    }
}

#[test]
fn dump_populated_blocks() {
    use fretwire_data::stream::{map_get, summarize, PresetStream};
    use rmpv::Value;

    let ps = PresetStream::parse(&blob()).unwrap();
    let slots = match ps.field(0).and_then(|m| map_get(m, 22)) {
        Some(Value::Array(a)) => a,
        _ => panic!("no block slots"),
    };
    eprintln!("===== populated block contents (type 6) =====");
    for (i, slot) in slots.iter().enumerate() {
        let ty = map_get(slot, 19).and_then(Value::as_i64);
        if ty == Some(6) {
            let content = map_get(slot, 20).unwrap();
            eprintln!("\n-- slot {i} (type {ty:?}) --\n{}", summarize(content, 3));
        }
    }
    // Also show the non-empty structural slots (types 0/1/2/3).
    for (i, slot) in slots.iter().enumerate() {
        let ty = map_get(slot, 19).and_then(Value::as_i64);
        if matches!(ty, Some(0) | Some(1) | Some(2) | Some(3)) {
            eprintln!("\n-- slot {i} STRUCTURAL (type {ty:?}) --\n{}",
                summarize(map_get(slot, 20).unwrap_or(&Value::Nil), 2));
        }
    }
}

#[test]
fn parses_preset_stream_structure() {
    use fretwire_data::stream::{map_get, value_bytes, PresetStream};
    use rmpv::Value;

    let ps = PresetStream::parse(&blob()).expect("parse preset stream");
    assert_eq!(ps.magic, "l6-helix");

    // Device-info field (key 7): { 36: model "P33", 35: version, 37: firmware }.
    let dev = ps.field(7).expect("device-info field 7");
    let model = map_get(dev, 36).and_then(value_bytes).unwrap();
    assert_eq!(
        std::str::from_utf8(model).unwrap().trim_end_matches('\0'),
        "P33", // HX Stomp's firmware family code
    );

    // Block-slots field (key 0 -> 22): an array of slot descriptors.
    let blocks = ps.field(0).and_then(|m| map_get(m, 22)).expect("block slots");
    let slots = match blocks {
        Value::Array(a) => a,
        _ => panic!("block slots not an array"),
    };
    assert_eq!(slots.len(), 20);
    // Populated effect blocks are type 6 with a parameter map; empties are type 8 / nil.
    let populated = slots
        .iter()
        .filter(|s| map_get(s, 19).and_then(Value::as_i64) == Some(6))
        .count();
    assert!(populated >= 5, "expected several populated blocks, got {populated}");
}

fn load_all_models() -> std::collections::HashMap<String, fretwire_data::Model> {
    let dir = data_dir();
    let mut by_name = std::collections::HashMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("models") {
            continue;
        }
        let file = fretwire_data::ModelFile::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for m in file.models {
            if let Some(name) = m.name.clone() {
                by_name.insert(name, m);
            }
        }
    }
    by_name
}

#[test]
fn set_slot_empty_matches_native_empty_and_keeps_bindings() {
    use fretwire_data::stream::{map_get, read_sequence, PresetStream};

    let mut ps = PresetStream::parse(&blob()).unwrap();
    let fs_before = map_get(&ps.preset, 3).cloned(); // footswitch layout
    let assign_before = map_get(&ps.preset, 4).cloned(); // assignments

    let slots = map_get(&ps.preset, 0).and_then(|g| map_get(g, 22)).unwrap();
    let rmpv::Value::Array(arr) = slots else { panic!("no slot array") };
    // A native empty slot is exactly {19: 8, 20: nil} — what set_slot_empty produces.
    let native_empty =
        arr.iter().find(|s| map_get(s, 19).and_then(|v| v.as_i64()) == Some(8)).cloned();
    let victim = arr.iter().position(|s| map_get(s, 19).and_then(|v| v.as_i64()) == Some(6)).unwrap();

    assert!(ps.set_slot_empty(victim));
    let our_empty = match map_get(&ps.preset, 0).and_then(|g| map_get(g, 22)) {
        Some(rmpv::Value::Array(a)) => a[victim].clone(),
        _ => panic!(),
    };
    if let Some(ne) = native_empty {
        assert_eq!(our_empty, ne, "our empty slot must match the device's native empty");
    }

    // Our serializer preserves the binding structures through a delete — so any binding loss the
    // device shows on a delete-write is device-side re-derivation, not our serialization.
    let re = ps.to_blob();
    let (seq, _) = read_sequence(&re, 3);
    assert_eq!(map_get(&seq[2], 3).cloned(), fs_before, "FS layout (key 3) must survive serialize");
    assert_eq!(map_get(&seq[2], 4).cloned(), assign_before, "assignments (key 4) must survive");
}

#[test]
fn to_blob_round_trips_the_preset() {
    use fretwire_data::stream::{locate_root, map_get, read_sequence, value_bytes, PresetStream};

    let data = blob();
    let ps = PresetStream::parse(&data).unwrap();

    // The original nested blob the device sent (read-stream key 104).
    let root = locate_root(&data, 64).unwrap();
    let orig = value_bytes(map_get(&root.value, 104).unwrap()).unwrap().to_vec();

    let re = ps.to_blob();

    // Diagnostic: how close are we to byte-identical? (Not required — the device parses msgpack.)
    if re == orig {
        eprintln!("to_blob is BYTE-IDENTICAL to the device blob ({} bytes)", re.len());
    } else {
        let at = re.iter().zip(&orig).position(|(a, b)| a != b);
        eprintln!("to_blob differs (re={} orig={} first diff @ {:?})", re.len(), orig.len(), at);
    }

    // Required: re-parsing our blob yields the same magic / header / preset tree.
    let (seq, _) = read_sequence(&re, 3);
    assert_eq!(seq.len(), 3, "blob should hold 3 values");
    assert_eq!(value_bytes(&seq[0]).unwrap(), format!("{}\0", ps.magic).as_bytes());
    assert_eq!(value_bytes(&seq[1]).unwrap(), ps.header.as_slice());
    assert_eq!(&seq[2], &ps.preset, "preset map must round-trip exactly");
}

#[test]
fn path_blocks_resolve_to_models() {
    use fretwire_data::stream::PresetStream;

    let ps = PresetStream::parse(&blob()).unwrap();
    let paths: Vec<_> = ps.footswitch_layout().into_iter().flatten().collect();
    let names: Vec<&str> = paths.iter().map(|p| p.model_name.as_str()).collect();
    eprintln!("path blocks: {names:?}");
    assert!(names.contains(&"Bucket Brigade"));
    assert!(names.contains(&"Harmonic Tremolo"));
    assert!(names.contains(&"Dynamic Hall"));

    // The Harmonic Tremolo block was renamed by the user to "Tremolo".
    let ht = paths.iter().find(|p| p.model_name == "Harmonic Tremolo").unwrap();
    assert_eq!(ht.user_label.as_deref(), Some("Tremolo"));

    // Bridge: device block model names resolve against the shipped .models database, giving us
    // the parameter definitions (names/ranges) for each block on the device.
    let by_name = load_all_models();
    let mut resolved = 0;
    for p in &paths {
        if let Some(model) = by_name.get(&p.model_name) {
            assert!(!model.params.is_empty(), "{} has no params", p.model_name);
            resolved += 1;
        }
    }
    eprintln!("resolved {resolved}/{} device blocks to .models definitions", paths.len());
    assert!(resolved >= 3, "expected to resolve most blocks, got {resolved}");
}

#[test]
fn typed_device_preset_model() {
    use fretwire_data::stream::{ParamValue, PresetStream};

    let ps = PresetStream::parse(&blob()).unwrap();
    assert_eq!(ps.device_model().as_deref(), Some("P33"));
    assert!(ps.firmware().unwrap().starts_with("v3.71"));

    let all = ps.blocks();
    assert_eq!(all.len(), 20);

    let effects = ps.effect_blocks();
    assert!(effects.len() >= 5, "got {} effect blocks", effects.len());

    // Every effect block carries an ordered param vector; values are floats/ints/bools.
    for b in &effects {
        assert!(!b.params.is_empty(), "effect block {} had no params", b.index);
    }
    // Spot-check a known block: slot 4 had a 10-value vector starting 1.9, 0.33, 3, ...
    let slot4 = all.iter().find(|b| b.index == 4).unwrap();
    assert_eq!(slot4.kind, 6);
    assert_eq!(slot4.params.len(), 10);
    assert_eq!(slot4.params[0], ParamValue::Float(1.9));
    assert_eq!(slot4.params[2], ParamValue::Int(3));
}

fn map_get(v: &rmpv::Value, key: i64) -> Option<&rmpv::Value> {
    if let rmpv::Value::Map(m) = v {
        m.iter().find(|(k, _)| k.as_i64() == Some(key)).map(|(_, val)| val)
    } else {
        None
    }
}

fn value_bytes(v: &rmpv::Value) -> Option<&[u8]> {
    match v {
        rmpv::Value::String(s) => Some(s.as_bytes()),
        rmpv::Value::Binary(b) => Some(b),
        _ => None,
    }
}
