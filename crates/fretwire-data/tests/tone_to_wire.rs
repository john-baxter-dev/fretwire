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
    tone::apply_tone,
};
use rmpv::Value;

/// The preset the oracle is built on: `FACTORY 1` slot 45.
const ORACLE_BANK: usize = 0;
const ORACLE_SLOT: usize = 45;

fn captures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/helix-floor")
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
    let report = apply_tone(&mut donor, &tone, &syms).expect("conversion");
    assert_eq!(report.blocks, 15, "every block in the tone should convert");
    Some((donor, oracle))
}

fn slots(ps: &PresetStream, dsp: i64) -> Vec<Value> {
    match map_get(&ps.preset, dsp).and_then(|m| map_get(m, 22)) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

/// Strip the parts of a slot the conversion deliberately does not write, so the comparison is over
/// exactly what it claims.
///
/// Those are the four **input/output** nodes, whose stored parameter vector is a ragged prefix of
/// the symbol's list — DSP1's input keeps 3 of its 8 parameters and DSP2's keeps none — and two
/// samples do not pin down the rule. Path A's pair are whole slots (0 = input, 9 = output); path
/// B's ride *inside* the structural slots, as key `14` of the split and key `16` of the mixer.
/// [solid — the split's `14 → 5` is the tone's `inputB.@input` and the mixer's `16 → 6` is
/// `outputB.@output`, on both DSPs]
fn claimed(index: usize, slot: &Value) -> Option<Value> {
    let strip = |slot: &Value, key: i64| {
        let mut slot = slot.clone();
        if let Some(Value::Map(content)) = map_get_mut(&mut slot, 20) {
            content.retain(|(k, _)| k.as_i64() != Some(key));
        }
        slot
    };
    match index {
        0 | 9 => None,               // input A, output A — whole slots we don't touch
        10 => Some(strip(slot, 14)), // the split, minus input B
        19 => Some(strip(slot, 16)), // the mixer, minus output B
        _ => Some(slot.clone()),
    }
}

/// `rmpv` has no public mutable map accessor, and the test needs one to strip a key.
fn map_get_mut(v: &mut Value, key: i64) -> Option<&mut Value> {
    match v {
        Value::Map(m) => m
            .iter_mut()
            .find(|(k, _)| k.as_i64() == Some(key))
            .map(|(_, v)| v),
        _ => None,
    }
}

/// The whole point: converting the backup's JSON reproduces the device's own preset, slot for slot.
///
/// This covers all 15 blocks across both DSPs — model index, paired cab, block class, enabled flag,
/// and every one of the 106 stored parameter values — plus the split and mixer nodes. A failure
/// here names the exact slot, which is what makes the mapping debuggable at all.
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
            assert_eq!(claimed(i, got), claimed(i, want), "dsp{dsp} slot {i}");
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
    let report = apply_tone(&mut donor, tone.as_object().unwrap(), &syms).unwrap();
    let said = report.not_carried.join("\n");
    for expected in ["snapshot", "footswitch", "irUuidTable", "input and output"] {
        assert!(
            said.contains(expected),
            "no mention of {expected} in:\n{said}"
        );
    }
}
