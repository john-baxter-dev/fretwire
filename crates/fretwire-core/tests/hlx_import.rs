//! Synthetic fixtures only: no proprietary model data or personal presets required.
use fretwire_core::backup::Backup;
use fretwire_core::hlx::{HlxPreset, validate_destination};
use fretwire_data::stream::{PresetStream, map_get};
use fretwire_data::symbols::DeviceSymbols;
use fretwire_data::tone::ValueTypes;
use rmpv::Value;
use serde_json::{Value as Json, json};

fn map(entries: Vec<(i64, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (Value::from(k), v))
            .collect(),
    )
}

fn donor(model: &str) -> PresetStream {
    let empty = map(vec![(19, Value::from(8)), (20, Value::Nil)]);
    let mut slots = vec![empty; 20];
    // Existing content must disappear, while undecoded settings must survive.
    slots[8] = map(vec![(19, Value::from(6)), (20, Value::Nil)]);
    for (slot, holder) in [(10, 15), (19, 17)] {
        slots[slot] = map(vec![
            (19, Value::from(if slot == 10 { 2 } else { 3 })),
            (
                20,
                map(vec![
                    (18, Value::from(true)),
                    (
                        holder,
                        map(vec![
                            (8, Value::from(0)),
                            (13, Value::from(3)),
                            (10, Value::from(true)),
                        ]),
                    ),
                ]),
            ),
        ]);
    }
    let snapshot = map(vec![
        (1, Value::from("undecoded")),
        (2, Value::Array(vec![Value::Nil; 10])),
        (3, Value::Array(vec![Value::Nil; 20])),
        (4, Value::from("OLD\0")),
    ]);
    let preset = map(vec![
        (
            0,
            map(vec![(21, Value::from(0)), (22, Value::Array(slots))]),
        ),
        (1, Value::Nil),
        (3, map(vec![(8, Value::Array(vec![Value::Nil; 5]))])),
        (4, Value::Array(vec![Value::Nil; 10])),
        (5, map(vec![(16, Value::F32(100.0)), (134, Value::from(2))])),
        (7, map(vec![(36, Value::from(format!("{model}\0")))])),
        (
            10,
            map(vec![
                (6, Value::from(0)),
                (8, Value::from(1)),
                (10, Value::Array(vec![snapshot; 3])),
            ]),
        ),
    ]);
    let mut blob = Vec::new();
    for value in [Value::from("l6-helix\0"), Value::from("hdr"), preset] {
        rmpv::encode::write_value(&mut blob, &value).unwrap();
    }
    let mut raw = vec![0; 8];
    rmpv::encode::write_value(&mut raw, &map(vec![(104, Value::Binary(blob))])).unwrap();
    PresetStream::parse(&raw).unwrap()
}

fn symbols() -> DeviceSymbols {
    DeviceSymbols::parse(
        br#"[
        {"symbol":"TestDriveMono","parameters":["Gain","HighCut"]},
        {"symbol":"HD2_AppDSPFlowSplitY","parameters":["BalanceA","BalanceB"]},
        {"symbol":"HD2_AppDSPFlowJoin","parameters":["Level"]}
    ]"#,
    )
    .unwrap()
}

fn file() -> Json {
    json!({"version": 6, "schema": "L6Preset", "data": {
        "device": 0x210006, "meta": {"name": "Imported"}, "tone": {
            "dsp0": {"block0": {"@model": "TestDrive", "@type": 0, "@path": 0,
                "@position": 0, "@enabled": false, "Gain": 0.6, "High Cut": 8000.0},
                "split": {"@model":"HD2_AppDSPFlowSplitY", "@position":0, "BalanceA":0.5, "BalanceB":0.5},
                "join": {"@model":"HD2_AppDSPFlowJoin", "@position":0, "Level":0.0}},
            "global": {"@topology0": "A", "@tempo": 120},
            "footswitch": {"dsp0": {"block0": {"@fs_index": 1, "@fs_label": "Drive"}}},
            "snapshot0": {"@name": "Verse", "@tempo": 110, "@valid": true,
                "blocks": {"dsp0": {"block0": true}}}
        }
    }})
}

#[test]
fn converts_without_mutating_donor_and_round_trips_existing_export_format() {
    let donor = donor("P33");
    let before = donor.to_stream(0);
    let prepared = HlxPreset::parse(&file().to_string())
        .unwrap()
        .prepare(&donor, &symbols(), &ValueTypes::default())
        .unwrap();
    assert_eq!(donor.to_stream(0), before);
    assert_eq!(prepared.name, "Imported");
    assert_eq!(
        (
            prepared.report.blocks,
            prepared.report.snapshots,
            prepared.report.footswitches
        ),
        (1, 1, 1)
    );
    assert!(prepared.require_acknowledgement(false).is_err());
    prepared.require_acknowledgement(true).unwrap();
    assert!(
        prepared
            .report
            .not_carried
            .iter()
            .any(|s| s.contains("donor"))
    );
    let raw = prepared.raw.clone();
    let out = PresetStream::parse(&raw).unwrap();
    let effects = out.effect_blocks();
    assert_eq!(effects.len(), 1, "the old donor block must be cleared");
    assert_eq!(effects[0].index, 1);
    assert_eq!(effects[0].bypassed, Some(true));
    assert_eq!(out.snapshots().1[0], "Verse");
    assert_eq!(map_get(out.field(5).unwrap(), 134), Some(&Value::from(2)));
    assert_eq!(map_get(out.field(5).unwrap(), 16), Some(&Value::F32(120.0)));
    assert_eq!(out.footswitch_layout()[0].len(), 1);
    let slots = map_get(out.field(0).unwrap(), 22)
        .unwrap()
        .as_array()
        .unwrap();
    for (slot, holder) in [(10, 15), (19, 17)] {
        let content = map_get(&slots[slot], 20).unwrap();
        assert_eq!(
            map_get(content, 18),
            Some(&Value::from(false)),
            "serial topology must deactivate donor's parallel nodes"
        );
        assert_eq!(
            map_get(map_get(content, holder).unwrap(), 13),
            Some(&Value::from(0))
        );
    }
    let backup = Backup::from_json(&prepared.into_backup().to_json()).unwrap();
    assert_eq!(backup.device, "HX Stomp");
    assert_eq!(backup.presets.len(), 1);
    assert_eq!(backup.presets[0].raw, raw);
}

#[test]
fn refuses_wrong_schema_device_name_and_dsp_before_conversion() {
    let cases: Vec<(&str, Json)> = vec![
        ("schema", json!("Other")),
        ("data.device", json!(0x210001)),
        ("data.meta.name", json!(" ")),
        ("data.meta.name", json!("bad\u{0}name")),
        ("data.tone.dsp0", Json::Null),
        ("data.tone.dsp1", json!({"block0": {}})),
    ];
    for (path, value) in cases {
        let mut f = file();
        let pointer = format!("/{}", path.replace('.', "/"));
        // dsp1 is absent in the base fixture.
        if path.ends_with("dsp1") {
            f["data"]["tone"]["dsp1"] = value;
        } else {
            *f.pointer_mut(&pointer).unwrap() = value;
        }
        assert!(HlxPreset::parse(&f.to_string()).is_err(), "{path}");
    }
    assert!(HlxPreset::parse("{").is_err());
}

#[test]
fn refuses_block_collisions_bad_positions_and_more_than_eight_blocks() {
    for (path, position) in [(-1, 0), (2, 0), (0, -1), (0, 8)] {
        let mut f = file();
        f["data"]["tone"]["dsp0"]["block0"]["@path"] = json!(path);
        f["data"]["tone"]["dsp0"]["block0"]["@position"] = json!(position);
        assert!(HlxPreset::parse(&f.to_string()).is_err());
    }
    let mut f = file();
    f["data"]["tone"]["dsp0"]["block1"] = f["data"]["tone"]["dsp0"]["block0"].clone();
    assert!(HlxPreset::parse(&f.to_string()).is_err());
    for i in 0..9 {
        let mut block = file()["data"]["tone"]["dsp0"]["block0"].clone();
        block["@path"] = json!(i / 8);
        block["@position"] = json!(i % 8);
        f["data"]["tone"]["dsp0"][format!("block{i}")] = block;
    }
    assert!(HlxPreset::parse(&f.to_string()).is_err());
}

#[test]
fn rejects_foreign_donor_unknown_model_and_incomplete_parameters() {
    let source = HlxPreset::parse(&file().to_string()).unwrap();
    assert!(
        source
            .prepare(&donor("P21"), &symbols(), &ValueTypes::default())
            .is_err()
    );
    for (field, value) in [
        ("@model", json!("Unknown")),
        ("Gain", Json::Null),
        ("Gain", json!([])),
    ] {
        let mut f = file();
        f["data"]["tone"]["dsp0"]["block0"][field] = value;
        assert!(
            HlxPreset::parse(&f.to_string())
                .unwrap()
                .prepare(&donor("P33"), &symbols(), &ValueTypes::default())
                .is_err()
        );
    }
    let mut f = file();
    f["data"]["tone"]["dsp0"]["block0"]
        .as_object_mut()
        .unwrap()
        .remove("Gain");
    assert!(
        HlxPreset::parse(&f.to_string())
            .unwrap()
            .prepare(&donor("P33"), &symbols(), &ValueTypes::default())
            .is_err()
    );
}

#[test]
fn reports_unknown_sections_and_checks_destination_bounds() {
    let mut f = file();
    f["data"]["tone"]["commandCenter"] = json!({"message": 1});
    let p = HlxPreset::parse(&f.to_string())
        .unwrap()
        .prepare(&donor("P33"), &symbols(), &ValueTypes::default())
        .unwrap();
    assert!(
        p.report
            .not_carried
            .iter()
            .any(|s| s.contains("commandCenter"))
    );
    for (bank, slot) in [(0, 0), (0, 125)] {
        validate_destination(bank, slot).unwrap();
    }
    for (bank, slot) in [(-1, 0), (1, 0), (0, -1), (0, 126)] {
        assert!(validate_destination(bank, slot).is_err());
    }
}

#[test]
fn paired_cabs_and_loopers_require_only_stored_parameters() {
    let symbols = DeviceSymbols::parse(
        br#"[
        {"symbol":"TestAmp","parameters":["Gain"]},
        {"symbol":"TestCab","parameters":["Cut","IrData"]},
        {"symbol":"TestLooperMono","parameters":["Playback","Reverse"]},
        {"symbol":"HD2_AppDSPFlowSplitY","parameters":["BalanceA","BalanceB"]},
        {"symbol":"HD2_AppDSPFlowJoin","parameters":["Level"]}
    ]"#,
    )
    .unwrap();
    let mut f = file();
    f["data"]["tone"]["dsp0"] = json!({
        "split": file()["data"]["tone"]["dsp0"]["split"],
        "join": file()["data"]["tone"]["dsp0"]["join"],
        "block0": {"@model":"TestAmp", "@type":3, "@path":0, "@position":0,
            "@cab":"cab0", "Gain":0.5},
        "cab0": {"@model":"TestCab", "Cut":8000.0},
        "block1": {"@model":"TestLooper", "@type":6, "@path":0, "@position":1,
            "Playback":0.8}
    });
    let source = HlxPreset::parse(&f.to_string()).unwrap();
    let p = source
        .prepare(&donor("P33"), &symbols, &ValueTypes::default())
        .unwrap();
    assert_eq!(p.report.blocks, 2);
    let out = PresetStream::parse(&p.raw).unwrap();
    let blocks: Vec<_> = out.blocks().into_iter().filter(|b| b.is_block()).collect();
    assert_eq!(blocks[0].paired_ref, Some(1));
    assert_eq!(blocks[0].paired_params.len(), 1, "IrData is not stored");
    assert_eq!(
        blocks[1].params.len(),
        1,
        "looper transport state is not stored"
    );
    f["data"]["tone"]["dsp0"]["cab0"]
        .as_object_mut()
        .unwrap()
        .remove("Cut");
    assert!(
        HlxPreset::parse(&f.to_string())
            .unwrap()
            .prepare(&donor("P33"), &symbols, &ValueTypes::default())
            .is_err()
    );
}

#[test]
fn refuses_incomplete_or_unsupported_routing_before_any_write() {
    for key in ["split", "join"] {
        let mut f = file();
        f["data"]["tone"]["dsp0"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(HlxPreset::parse(&f.to_string()).is_err());
    }
    for topology in [Json::Null, json!("AB"), json!("SAB"), json!("unknown")] {
        let mut f = file();
        f["data"]["tone"]["global"]["@topology0"] = topology;
        assert!(HlxPreset::parse(&f.to_string()).is_err());
    }
    let mut f = file();
    f["data"]["tone"]["dsp0"]["split"]
        .as_object_mut()
        .unwrap()
        .remove("BalanceA");
    assert!(
        HlxPreset::parse(&f.to_string())
            .unwrap()
            .prepare(&donor("P33"), &symbols(), &ValueTypes::default())
            .is_err()
    );
}

#[test]
fn readback_verification_detects_a_discarded_dsp_and_missing_assignments() {
    let prepared = HlxPreset::parse(&file().to_string())
        .unwrap()
        .prepare(&donor("P33"), &symbols(), &ValueTypes::default())
        .unwrap();
    let expected = PresetStream::parse(&prepared.raw).unwrap();
    prepared.verify_readback(&expected).unwrap();
    let mut reordered = expected.clone();
    fn reverse_maps(value: &mut Value) {
        match value {
            Value::Map(entries) => {
                entries.reverse();
                for (_, value) in entries {
                    reverse_maps(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    reverse_maps(value);
                }
            }
            _ => {}
        }
    }
    reverse_maps(reordered.preset_mut());
    prepared.verify_readback(&reordered).unwrap();
    for field in [0, 3, 4, 10] {
        let mut actual = expected.clone();
        let Value::Map(entries) = actual.preset_mut() else {
            panic!("expected map")
        };
        entries
            .iter_mut()
            .find(|(key, _)| key.as_i64() == Some(field))
            .unwrap()
            .1 = Value::Nil;
        let error = prepared.verify_readback(&actual).unwrap_err().to_string();
        assert!(error.contains("has not been saved"));
    }
}

#[test]
fn permits_regenerated_switch_display_metadata_but_checks_custom_labels() {
    let prepared = HlxPreset::parse(&file().to_string())
        .unwrap()
        .prepare(&donor("P33"), &symbols(), &ValueTypes::default())
        .unwrap();
    let mut actual = PresetStream::parse(&prepared.raw).unwrap();
    fn field(value: &mut Value, key: i64) -> &mut Value {
        let Value::Map(entries) = value else {
            panic!("map expected")
        };
        &mut entries
            .iter_mut()
            .find(|(k, _)| k.as_i64() == Some(key))
            .unwrap()
            .1
    }
    let Value::Array(switches) = field(field(actual.preset_mut(), 3), 8) else {
        panic!("switches")
    };
    let Value::Array(bindings) = &mut switches[0] else {
        panic!("bindings")
    };
    let node = field(&mut bindings[0], 11);
    *field(node, 5) = Value::from("Regenerated model name\0");
    *field(node, 6) = Value::from(16711683);
    prepared.verify_readback(&actual).unwrap();
    let Value::Array(switches) = field(field(actual.preset_mut(), 3), 8) else {
        panic!("switches")
    };
    let Value::Array(bindings) = &mut switches[0] else {
        panic!("bindings")
    };
    *field(&mut bindings[0], 14) = Value::from("Lost custom label\0");
    assert!(prepared.verify_readback(&actual).is_err());
}

#[test]
fn snapshot_controller_count_is_rebuilt_instead_of_inherited_from_donor() {
    let mut f = file();
    f["data"]["tone"]["controller"] = json!({"dsp0":{"block0":{
        "Gain":{"@controller":9,"@min":0.0,"@max":1.0},
        "HighCut":{"@controller":1,"@min":1000.0,"@max":8000.0}
    }}});
    f["data"]["tone"]["snapshot0"]["controllers"] = json!({"dsp0":{"block0":{
        "Gain":{"@value":0.7}, "HighCut":{"@value":6000.0}
    }}});
    let prepared = HlxPreset::parse(&f.to_string())
        .unwrap()
        .prepare(&donor("P33"), &symbols(), &ValueTypes::default())
        .unwrap();
    let converted = PresetStream::parse(&prepared.raw).unwrap();
    assert_eq!(prepared.report.controllers, 2);
    assert_eq!(
        map_get(converted.field(10).unwrap(), 8),
        Some(&Value::from(2))
    );
    let cleared = HlxPreset::parse(&file().to_string())
        .unwrap()
        .prepare(&converted, &symbols(), &ValueTypes::default())
        .unwrap();
    let cleared = PresetStream::parse(&cleared.raw).unwrap();
    assert_eq!(
        map_get(cleared.field(10).unwrap(), 8),
        Some(&Value::from(0))
    );
}

#[test]
fn source_snapshot_selection_is_separate_from_controller_count() {
    let mut f = file();
    f["data"]["tone"]["global"]["@current_snapshot"] = json!(2);
    assert!(
        HlxPreset::parse(&f.to_string()).is_err(),
        "missing selected scene"
    );
    f["data"]["tone"]["snapshot2"] = f["data"]["tone"]["snapshot0"].clone();
    let prepared = HlxPreset::parse(&f.to_string())
        .unwrap()
        .prepare(&donor("P33"), &symbols(), &ValueTypes::default())
        .unwrap();
    assert_eq!(prepared.active_snapshot, 2);
    let p = PresetStream::parse(&prepared.raw).unwrap();
    assert_eq!(p.snapshots().0, Some(2));
    assert_eq!(map_get(p.field(10).unwrap(), 8), Some(&Value::from(0)));
    f["data"]["tone"]["global"]["@current_snapshot"] = json!(3);
    assert!(HlxPreset::parse(&f.to_string()).is_err());
}
