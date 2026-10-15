//! `tone` JSON → wire preset, checked against one preset held in **both** forms.
//!
//! A contributor's Helix Floor gave us its `.hxb` backup on 2026-07-22 and, fifteen hours later, a
//! USB capture of HX Edit opening `FACTORY 1` slot 45 ("Pull Me Under") off the same unit. So the
//! same preset exists as host-side `tone` JSON *and* as the integer-keyed MessagePack the device
//! actually sends — which makes the conversion checkable rather than merely plausible: convert the
//! JSON, and the device's own bytes say whether it is right.
//!
//! The conversion is done onto a **different** preset ("Sultans", `FACTORY 2` slot 64, from the
//! same capture) so nothing can pass by being left over from the donor. Every block slot,
//! every parameter value and both split topologies must come out equal to the real thing.
//!
//! **None of these inputs are in git.** `captures/helix-floor/` is a contributor's own preset data
//! and is ignored wholesale, and the reference data (`Helix.sym`) is Line 6's. The test skips when
//! either is absent, so a clean clone stays green. To regenerate the two streams from the capture:
//!
//! ```text
//! tools/extract-preset-stream.py captures/helix-floor/WinCap5.pcapng captures/helix-floor/wc
//! mv captures/helix-floor/wc0.msgpack.bin captures/helix-floor/floor-sultans.msgpack.bin
//! mv captures/helix-floor/wc1.msgpack.bin captures/helix-floor/floor-pullmeunder.msgpack.bin
//! ```
#![cfg(have_bundled_data)]

use std::path::PathBuf;

use fretwire_data::{
    hxb::Hxb,
    stream::{PresetStream, map_get},
    symbols::DeviceSymbols,
    tone::{ValueTypes, apply_tone},
};
use rmpv::Value;

/// The preset the oracle is built on: `FACTORY 1` slot 45.
const ORACLE_BANK: usize = 0;
const ORACLE_SLOT: usize = 45;

fn captures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/helix-floor")
}

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("FRETWIRE_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
                .map(|b| b.join("fretwire").join("data"))
        })
}

fn symbols() -> Option<DeviceSymbols> {
    DeviceSymbols::parse(&std::fs::read(data_dir()?.join("Helix.sym")).ok()?).ok()
}

/// The family's `.models` value types, which decide how a `tone` number is encoded.
fn types() -> ValueTypes {
    let dir = data_dir();
    let files: Vec<Vec<u8>> = dir
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "models"))
        .filter_map(|p| std::fs::read(p).ok())
        .collect();
    ValueTypes::from_model_files(files.iter().map(Vec::as_slice))
}

/// The contributor's backup, whatever it is called — the file name carries its date.
fn backup() -> Option<Hxb> {
    let hxb = std::fs::read_dir(captures())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "hxb"))?;
    Hxb::parse(&std::fs::read(hxb).ok()?).ok()
}

fn stream(name: &str) -> Option<PresetStream> {
    PresetStream::parse(&std::fs::read(captures().join(name)).ok()?).ok()
}

/// Convert the backup's copy of the preset onto the unrelated donor, and hand back both that and
/// the device's own version of the same preset.
fn converted_and_oracle() -> Option<(PresetStream, PresetStream)> {
    let syms = symbols()?;
    let oracle = stream("floor-pullmeunder.msgpack.bin")?;
    let mut donor = stream("floor-sultans.msgpack.bin")?;
    let hxb = backup()?;
    let tone = hxb
        .setlists()
        .get(ORACLE_BANK)?
        .presets
        .get(ORACLE_SLOT)?
        .clone()?
        .tone;
    let tone = tone.as_object()?.clone();
    let report = apply_tone(&mut donor, &tone, &syms, &types()).expect("conversion");
    assert_eq!(report.blocks, 15, "every block in the tone should convert");
    Some((donor, oracle))
}

fn slots(ps: &PresetStream, dsp: i64) -> Vec<Value> {
    match map_get(&ps.preset, dsp).and_then(|m| map_get(m, 22)) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

/// The whole point: converting the backup's JSON reproduces the device's own preset, slot for slot.
///
/// This covers all 15 blocks across both DSPs — model index, paired cab, block class, enabled flag,
/// and every one of the 106 stored parameter values — plus the split and mixer nodes and, since
/// 2026-10-14, the four input/output nodes, so every slot is compared whole. A failure here names
/// the exact slot, which is what makes the mapping debuggable at all.
#[test]
fn converting_the_backup_reproduces_the_device_preset() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        eprintln!("skipping: needs the contributor's Floor capture + backup");
        return;
    };
    for dsp in 0..2i64 {
        let (got, want) = (slots(&converted, dsp), slots(&oracle, dsp));
        assert_eq!(got.len(), want.len(), "dsp{dsp}: slot array length");
        for (i, (got, want)) in got.iter().zip(&want).enumerate() {
            assert_eq!(got, want, "dsp{dsp} slot {i}");
        }
    }
}

/// Both DSPs' split type comes across, including the case a single-DSP device cannot produce: a
/// bracket that opens on DSP1 (`SAB` → 2) and closes on DSP2 (`ABJ` → 3).
#[test]
fn the_split_topology_comes_across() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        return;
    };
    for dsp in 0..2i64 {
        let split_type = |ps: &PresetStream| {
            map_get(&ps.preset, dsp)
                .and_then(|m| map_get(m, 21))
                .and_then(Value::as_i64)
        };
        assert_eq!(
            split_type(&converted),
            split_type(&oracle),
            "dsp{dsp} key 21"
        );
    }
    assert_eq!(
        (
            map_get(&converted.preset, 0)
                .and_then(|m| map_get(m, 21))
                .and_then(Value::as_i64),
            map_get(&converted.preset, 1)
                .and_then(|m| map_get(m, 21))
                .and_then(Value::as_i64),
        ),
        (Some(2), Some(3)),
        "the bracket spans both DSPs, which is why this preset is the one worth testing"
    );
}

/// The donor's blocks must not survive. "Sultans" is a serial 8-block preset and "Pull Me Under" is
/// a 15-block parallel one, so a slot the conversion forgot to clear shows up as a block the tone
/// never asked for — the failure mode that a same-preset round-trip would hide entirely.
#[test]
fn nothing_leaks_from_the_donor() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        return;
    };
    let occupied = |ps: &PresetStream| {
        (0..2i64)
            .flat_map(|d| {
                slots(ps, d)
                    .into_iter()
                    .enumerate()
                    .map(move |(i, s)| (d, i, s))
            })
            .filter(|(_, _, s)| map_get(s, 19).and_then(Value::as_i64) == Some(6))
            .map(|(d, i, _)| (d, i))
            .collect::<Vec<_>>()
    };
    assert_eq!(occupied(&converted), occupied(&oracle));
}

/// Snapshots come across: name, tempo, appearance, and the per-slot bypass matrix.
///
/// The matrix is the part worth pinning. It is indexed by **wire slot** across both DSPs — 40
/// entries on a Floor, not one array per DSP — while the tone keys it by block name, so every
/// entry has to be re-indexed through the same `@path`/`@position` mapping the blocks use. Getting
/// that wrong is silent: the chain looks right and the snapshots all switch to the wrong scene.
/// All eight snapshots must match, which is 320 cells.
#[test]
fn snapshots_come_across() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        return;
    };
    let snapshots = |ps: &PresetStream| match map_get(&ps.preset, 10).and_then(|m| map_get(m, 10)) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    let (got, want) = (snapshots(&converted), snapshots(&oracle));
    assert_eq!(got.len(), 8, "the Floor holds eight snapshots");
    assert_eq!(got.len(), want.len());
    for (i, (got, want)) in got.iter().zip(&want).enumerate() {
        // Keys 1 and 2 are controller state, keyed off an assignment table this does not write.
        for key in [0i64, 3, 4, 5, 11, 12, 14] {
            assert_eq!(
                map_get(got, key),
                map_get(want, key),
                "snapshot {i} key {key}"
            );
        }
    }
}

/// The footswitch layout comes across — bindings, custom labels and LED ring colours.
///
/// Every **bypass** binding must match the device's own, position for position. The two
/// interesting entries are the switch carrying two blocks (Volume Pedal and Weeper both on FS13,
/// ordered primary-first with `10` recording the order) and the two whose `@fs_enabled` is false —
/// bound but not currently answering, which the array still holds.
///
/// The one position that legitimately differs is FS8, which the device fills with a **parameter
/// controller** (`11 → 0` = 2, the split's `Route To`). That is a row of key `4` as well as of
/// this table, and the conversion writes neither, so writing it here alone would leave the two
/// disagreeing.
#[test]
fn the_footswitch_layout_comes_across() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        return;
    };
    let layout = |ps: &PresetStream| match map_get(&ps.preset, 3).and_then(|m| map_get(m, 8)) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    let (got, want) = (layout(&converted), layout(&oracle));
    assert_eq!(got.len(), want.len(), "the array keeps the device's width");

    // Whole-array: the bypass bindings from the tone's `footswitch` section and the type-2 row
    // its `controller` section puts on a switch (the split's "Route To") land in the same array,
    // and every position must be the device's own bytes. (This test used to carve out the type-2
    // switch; the carve-out died when apply_controllers learned to write it.)
    let mut bound = 0;
    for (i, (got, want)) in got.iter().zip(&want).enumerate() {
        assert_eq!(got, want, "FS{}", i + 1);
        if !matches!(want, Value::Nil) {
            bound += 1;
        }
    }
    assert_eq!(
        bound, 9,
        "nine switches carry a binding, one of them a controller"
    );
}

/// The donor's controller assignments are **cleared**, not kept: key `4` addresses blocks by slot
/// and by parameter index, and every slot has just been rewritten, so a surviving row would sweep
/// a parameter of whatever now sits there.
#[test]
fn controller_assignments_come_across() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        return;
    };
    let table = |ps: &PresetStream| match map_get(&ps.preset, 4) {
        Some(Value::Array(a)) => a.clone(),
        other => panic!("no assignment table: {other:?}"),
    };
    let (got, want) = (table(&converted), table(&oracle));
    assert_eq!(got.len(), want.len(), "the array itself must survive");

    // The tone carries four assignments — the wah and volume pedals on EXP1/EXP2, a footswitch
    // on the split's Route To, and the snapshots source on a DSP2 drive — and every one must
    // reproduce the device's own row byte for byte, places included.
    for (ordinal, (row, expect)) in got.iter().zip(&want).enumerate() {
        assert_eq!(row, expect, "ordinal {ordinal}");
    }

    // The snapshots' per-controller values (key 2) index by those places, so each snapshot's
    // whole array must match the device's — the four written rows and the unused-row sentinel
    // everywhere else.
    let snapshots = |ps: &PresetStream| match map_get(&ps.preset, 10).and_then(|m| map_get(m, 10)) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    for (i, (got, want)) in snapshots(&converted)
        .iter()
        .zip(&snapshots(&oracle))
        .enumerate()
    {
        assert_eq!(map_get(got, 2), map_get(want, 2), "snapshot {i} values");
    }
}

/// A conversion says what it left behind. This preset carries eight snapshots, footswitch
/// bindings with custom labels and colours, and an IR table — none of which this writes yet, and
/// all of which a caller has to be able to tell the user about.
#[test]
fn the_report_names_what_it_did_not_carry() {
    let Some((_, _)) = converted_and_oracle() else {
        return;
    };
    let syms = symbols().unwrap();
    let mut donor = stream("floor-sultans.msgpack.bin").unwrap();
    let hxb = backup().unwrap();
    let tone = hxb.setlists()[ORACLE_BANK].presets[ORACLE_SLOT]
        .clone()
        .unwrap()
        .tone;
    let report = apply_tone(&mut donor, tone.as_object().unwrap(), &syms, &types()).unwrap();
    let said = report.not_carried.join("\n");
    for expected in ["snapshot", "controller assignments", "irUuidTable"] {
        assert!(
            said.contains(expected),
            "no mention of {expected} in:\n{said}"
        );
    }
    // The Helix's input/output nodes are written now, so the report must not claim otherwise.
    assert!(
        !said.contains("input and output"),
        "the I/O nodes are carried:\n{said}"
    );
}

/// Topology `AB`, a DSP the two paths only pass through, from a second preset held in both forms:
/// the Floor backup's `tone` of the factory "Plush Garden" (`FACTORY 1` slot 47, `SAB`/`AB`) and a
/// Helix Rack owner's dump of the same factory preset (issue #24). Both units report `P21`.
///
/// The dump is in `captures/helix-rack/`, ignored like the Floor's; the test skips without it.
#[test]
fn topology_ab_comes_across() {
    let Some(syms) = symbols() else {
        return;
    };
    let (Some(oracle), Some(mut donor), Some(hxb)) = (
        PresetStream::parse(
            &std::fs::read(captures().join("../helix-rack/rack-plushgarden.msgpack.bin"))
                .unwrap_or_default(),
        )
        .ok(),
        stream("floor-sultans.msgpack.bin"),
        backup(),
    ) else {
        eprintln!("skipping: needs the Rack's Plush Garden dump + the Floor capture and backup");
        return;
    };
    let tone = hxb.setlists()[0].presets[47]
        .clone()
        .expect("Plush Garden")
        .tone;
    let tone = tone.as_object().unwrap().clone();
    assert_eq!(
        tone["global"]["@topology1"], "AB",
        "the backup's slot 47 should be Plush Garden"
    );
    let report = apply_tone(&mut donor, &tone, &syms, &types()).expect("conversion");
    assert!(
        !report.not_carried.iter().any(|n| n.contains("topology")),
        "every topology should carry: {:?}",
        report.not_carried
    );
    let split_type = |ps: &PresetStream, dsp: i64| {
        map_get(&ps.preset, dsp)
            .and_then(|m| map_get(m, 21))
            .and_then(Value::as_i64)
    };
    assert_eq!(
        (split_type(&donor, 0), split_type(&donor, 1)),
        (split_type(&oracle, 0), split_type(&oracle, 1)),
    );
    assert_eq!(
        (split_type(&oracle, 0), split_type(&oracle, 1)),
        (Some(2), Some(4))
    );
}

/// `a == b`, except that two `f32`s may differ in the sixth significant figure. HX Edit's JSON does
/// not always round-trip the device's float: this Rack's own backup has Plush Garden's gate decay
/// as `0.1499999761581421` where the Rack stores `0.15`, and its worst value across three presets
/// is a predelay of `0.011000037` against a stored `0.011` (40 ULP, 3e-6 relative). Types must
/// still match exactly, so an integer written where the device has an `f32` fails.
fn same_to_float_noise(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Map(x), Value::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((ka, va), (kb, vb))| ka == kb && same_to_float_noise(va, vb))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_to_float_noise(a, b))
        }
        (Value::F32(x), Value::F32(y)) => (x - y).abs() <= y.abs().max(1.0) * 1e-5,
        _ => a == b,
    }
}

/// Three more presets held in both forms, all from **one Helix Rack**: its owner's HX Edit backup
/// and `dump-raw`s of the same presets (issue #24, 2026-10-14). Each converts onto a different one
/// of the three as donor, and every slot of both DSPs, the four input/output nodes included, must
/// come out as the device stored it, along with the preset tempo and the focused block. This
/// backup writes whole-number knobs as JSON integers, so it also pins the `valueType` typing. Pull Me Under is a cross-DSP bracket (`SAB`/`ABJ`), Plush
/// Garden passes through DSP2 (`SAB`/`AB`) and US Double Nrm is serial.
///
/// Untracked like the Floor's: `captures/helix-rack/` holds the owner's files. Skips without them.
#[test]
fn three_rack_presets_convert_to_what_the_rack_stored() {
    let rack = captures().join("../helix-rack");
    let (Some(syms), Ok(bytes)) = (
        symbols(),
        std::fs::read(rack.join("Helix Rack Backup 2026-Oct-14.hxb")),
    ) else {
        eprintln!("skipping: needs the Rack backup and dumps in captures/helix-rack");
        return;
    };
    let hxb = Hxb::parse(&bytes).expect("Rack backup");
    let read = |name: &str| {
        PresetStream::parse(&std::fs::read(rack.join(name)).expect("Rack dump")).expect("parse")
    };
    for (bank, index, oracle, donor) in [
        (
            0,
            53,
            "rack-plushgarden.msgpack.bin",
            "rack-pullmeunder.msgpack.bin",
        ),
        (
            1,
            83,
            "rack-pullmeunder.msgpack.bin",
            "rack-usdoublenrm.msgpack.bin",
        ),
        (
            0,
            0,
            "rack-usdoublenrm.msgpack.bin",
            "rack-plushgarden.msgpack.bin",
        ),
    ] {
        let oracle = read(oracle);
        let mut donor = read(donor);
        let tone = hxb.setlists()[bank].presets[index]
            .clone()
            .expect("a preset")
            .tone;
        apply_tone(&mut donor, tone.as_object().unwrap(), &syms, &types()).expect("conversion");
        // The preset tempo and the focused block, out of the tone's `global`.
        for (group, key) in [(5, 16), (6, 98)] {
            let at = |ps: &PresetStream| {
                map_get(&ps.preset, group)
                    .and_then(|g| map_get(g, key))
                    .cloned()
            };
            assert_eq!(at(&donor), at(&oracle), "{bank}/{index} /{group}/{key}");
        }
        for dsp in 0..2i64 {
            let (got, want) = (slots(&donor, dsp), slots(&oracle, dsp));
            assert_eq!(
                got.len(),
                want.len(),
                "{bank}/{index} dsp{dsp}: slot array length"
            );
            for (i, (got, want)) in got.iter().zip(&want).enumerate() {
                assert!(
                    same_to_float_noise(got, want),
                    "{bank}/{index} dsp{dsp} slot {i}:\n  got {got:?}\n want {want:?}"
                );
            }
        }
    }
}
