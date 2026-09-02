//! POD Go tone-JSON → wire conversion, against the strongest oracle available: the owner's `.pgb`
//! backup and the same unit's own wire stream of the **same preset** (Factory 01A, "US Deluxe
//! Nrm"), both from 2026-08-27 (issue #15).
//!
//! The Floor's version of this test (`tone_to_wire.rs`) uses an unrelated donor so a leak shows
//! up as an extra block. Only one POD Go wire preset exists, so the donor here *is* the oracle —
//! instead, every region the conversion claims to write is **vandalised first** (blocks emptied,
//! layout and controller table nulled, snapshots renamed). Equality with the pristine oracle then
//! means the conversion actually rewrote all of it from the JSON; anything it forgot stays
//! vandalised and fails loudly.
//!
//! Personal device data: both files stay out of git, so everything here skips on a clean clone.

use fretwire_data::{
    hxb::Hxb,
    stream::{PresetStream, map_get},
    symbols::DeviceSymbols,
    tone::apply_tone,
};
use rmpv::Value;
use std::path::PathBuf;

fn captures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/pod-go")
}

/// The POD Go's own symbol table, from the imported reference data.
fn symbols() -> Option<DeviceSymbols> {
    let dir = std::env::var_os("FRETWIRE_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
                .map(|b| b.join("fretwire").join("data"))
        })?;
    DeviceSymbols::parse(&std::fs::read(dir.join("pod-go/PodGo.sym")).ok()?).ok()
}

fn backup() -> Option<Hxb> {
    let pgb = std::fs::read_dir(captures().join("backup"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "pgb"))?;
    Hxb::parse(&std::fs::read(pgb).ok()?).ok()
}

fn oracle() -> Option<PresetStream> {
    PresetStream::parse(&std::fs::read(captures().join("usdeluxe-factory01a.msgpack.bin")).ok()?)
        .ok()
}

fn map_get_mut(v: &mut Value, key: i64) -> Option<&mut Value> {
    match v {
        Value::Map(m) => m
            .iter_mut()
            .find(|(k, _)| k.as_i64() == Some(key))
            .map(|(_, v)| v),
        _ => None,
    }
}

/// Wreck every region `apply_tone` claims to write, so the donor's own bytes cannot pass for a
/// successful conversion.
fn vandalise(donor: &mut PresetStream) {
    // The ten block slots (1..=10): emptied.
    for slot in 1..=10 {
        donor.set_slot_empty(slot);
    }
    // The footswitch layout (3 → 8): every switch cleared.
    if let Some(Value::Array(switches)) =
        map_get_mut(&mut donor.preset, 3).and_then(|l| map_get_mut(l, 8))
    {
        switches.fill(Value::Nil);
    }
    // The controller table (4): every row cleared.
    if let Some(Value::Array(rows)) = map_get_mut(&mut donor.preset, 4) {
        rows.fill(Value::Nil);
    }
    // The snapshots (10 → 10): names and bypass matrices scribbled over.
    if let Some(Value::Array(snapshots)) =
        map_get_mut(&mut donor.preset, 10).and_then(|g| map_get_mut(g, 10))
    {
        for snapshot in snapshots.iter_mut() {
            if let Some(name) = map_get_mut(snapshot, 4) {
                *name = Value::from("VANDALISED\0");
            }
            if let Some(Value::Array(cells)) = map_get_mut(snapshot, 3) {
                for cell in cells.iter_mut() {
                    *cell = Value::Array(vec![Value::from(false), Value::from(false)]);
                }
            }
        }
    }
}

fn converted_and_oracle() -> Option<(PresetStream, PresetStream)> {
    let syms = symbols()?;
    let oracle = oracle()?;
    let mut donor = oracle.clone();
    vandalise(&mut donor);
    let tone = backup()?.setlists().first()?.presets.first()?.clone()?.tone;
    let tone = tone.as_object()?.clone();
    let report = apply_tone(&mut donor, &tone, &syms).expect("conversion");
    assert_eq!(report.blocks, 10, "all ten fixed-chain blocks convert");
    for line in &report.not_carried {
        eprintln!("not carried: {line}");
    }
    Some((donor, oracle))
}

/// Value equality at the `.pgb`'s own precision. POD Go Edit **rounds parameter values to three
/// decimals** when it writes a backup (`"Tone" : 0.270` against the wire's f32 `0.26999998`), so
/// bit-exact floats are unrecoverable from the file — for anyone, its own restore included. Every
/// non-float leaf must still match exactly.
fn similar(a: &Value, b: &Value) -> bool {
    let as_f = |v: &Value| match v {
        Value::F32(f) => Some(*f as f64),
        Value::F64(f) => Some(*f),
        _ => None,
    };
    match (a, b) {
        (Value::Map(x), Value::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((ka, va), (kb, vb))| ka == kb && similar(va, vb))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| similar(a, b))
        }
        _ => match (as_f(a), as_f(b)) {
            (Some(x), Some(y)) => (x - y).abs() <= f64::max(5e-4, y.abs() * 1e-6),
            _ => a == b,
        },
    }
}

/// Converting the backup's JSON reproduces the device's own preset, slot for slot — model index,
/// block class, enabled flag and every stored parameter value (at the backup's three-decimal
/// precision), through the POD Go's pathless `@position` addressing.
#[test]
fn converting_the_pgb_reproduces_the_device_preset() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        eprintln!("skipping: needs the POD Go capture + backup + reference data");
        return;
    };
    let slots = |ps: &PresetStream| match map_get(&ps.preset, 0).and_then(|m| map_get(m, 22)) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    let (got, want) = (slots(&converted), slots(&oracle));
    assert_eq!(got.len(), want.len(), "slot array length");
    for (i, (got, want)) in got.iter().zip(&want).enumerate() {
        // 0 and 11 are the input/output nodes the conversion leaves alone (and never vandalised).
        assert!(
            similar(got, want),
            "slot {i}:\n  got {got:?}\n want {want:?}"
        );
    }
}

/// The footswitch layout comes back whole — including the toe-switch pair at position 8, where
/// the wah and the volume pedal share one switch with one of them enabled.
#[test]
fn the_footswitch_layout_comes_across() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        return;
    };
    let layout = |ps: &PresetStream| match map_get(&ps.preset, 3).and_then(|l| map_get(l, 8)) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    assert_eq!(layout(&converted), layout(&oracle));
}

/// The controller table and all four snapshots come back byte-equal, matrices included.
#[test]
fn controllers_and_snapshots_come_across() {
    let Some((converted, oracle)) = converted_and_oracle() else {
        return;
    };
    let similar_at = |key: i64| {
        let (got, want) = (
            map_get(&converted.preset, key),
            map_get(&oracle.preset, key),
        );
        match (got, want) {
            (Some(g), Some(w)) => {
                assert!(similar(g, w), "key {key}:\n  got {g:?}\n want {w:?}")
            }
            _ => panic!("key {key} missing on one side"),
        }
    };
    similar_at(4); // controller table
    similar_at(10); // snapshots, matrices included
}

/// The second oracle pair: "AC30 Ambient" (User 04A, backup `SL01[12]`) — the preset the very
/// first POD Go capture streamed (2026-08-25). It adds the classes the Factory preset lacks:
/// a distortion, the Simple 3-Band EQ (class 23's second sample), a delay at class 8 with its
/// `@trails` extra — and the **impulse response**.
///
/// The IR converts in POD Go Edit's own op-21 shape (five symbol parameters, bare uuid — see
/// `encode_block`), so its slot compares against the device stream with the measured writer
/// differences pinned: the device's read-back appends a sixth, device-generated value and
/// NUL-terminates the uuid. `Index` agrees here (both `7`, the IR's library slot), but it is
/// the uuid that binds — it is the MD5 of the IR's raw samples (hashing the backup's own `I006`
/// WAV section produces exactly this preset's uuid), and the second-IR capture shows the pedal
/// re-resolving a stale backup `Index: 41` to the live slot `6` by that hash.
#[test]
fn the_second_preset_reproduces_too_including_the_ir() {
    let (Some(syms), Some(hxb), Some(oracle)) = (
        symbols(),
        backup(),
        PresetStream::parse(
            &std::fs::read(captures().join("ac30ambient-user04a.msgpack.bin")).unwrap_or_default(),
        )
        .ok(),
    ) else {
        eprintln!("skipping: needs the POD Go captures + backup + reference data");
        return;
    };
    let tone = hxb.setlists()[1].presets[12]
        .clone()
        .expect("SL01[12]")
        .tone;
    let tone = tone.as_object().expect("tone object").clone();

    let mut donor = oracle.clone();
    vandalise(&mut donor);
    let report = apply_tone(&mut donor, &tone, &syms).expect("conversion");
    assert_eq!(report.blocks, 10, "all ten blocks convert, the IR included");
    let slots = |ps: &PresetStream| match map_get(&ps.preset, 0).and_then(|m| map_get(m, 22)) {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    for (i, (got, want)) in slots(&donor).iter().zip(&slots(&oracle)).enumerate() {
        if i == 7 {
            // The IR slot: pin the writer differences, then compare the rest.
            let bank = |v: &Value| map_get(v, 20).and_then(|c| map_get(c, 11)).cloned();
            let (Some(got_bank), Some(want_bank)) = (bank(got), bank(want)) else {
                panic!("IR slot has no param bank");
            };
            let values = |b: &Value| match map_get(b, 4) {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            let (gv, wv) = (values(&got_bank), values(&want_bank));
            assert_eq!((gv.len(), wv.len()), (5, 6), "five written, six read back");
            assert_eq!(wv[5].as_i64(), Some(6), "the device-generated extra");
            assert_eq!(gv[0].as_i64(), Some(7), "the backup's save-time Index");
            assert_eq!(
                wv[0].as_i64(),
                Some(7),
                "the live library slot — agreeing here"
            );
            for (a, b) in gv[1..5].iter().zip(&wv[1..5]) {
                assert!(similar(a, b), "IR param: {a:?} vs {b:?}");
            }
            let uuid = |v: &Value| {
                map_get(v, 20)
                    .and_then(|c| map_get(c, 27))
                    .and_then(Value::as_str)
                    .map(|s| s.trim_end_matches('\0').to_string())
            };
            assert_eq!(uuid(got), uuid(want), "the uuid binds, modulo termination");
            assert_eq!(
                uuid(got).as_deref(),
                Some("48b188d8be8f7fca84c097e9be82dd01")
            );
            let class = |v: &Value| map_get(v, 20).and_then(|c| map_get(c, 9)).cloned();
            assert_eq!(class(got), class(want), "class 15 both sides");
            continue;
        }
        if i == 4 {
            // The one known drift between the two source artifacts: the wire stream (08-25) has
            // the Minotaur engaged, the backup (08-27) has it bypassed — its owner was stomping
            // switches between the two. Everything else in the slot must still match; pinning
            // the flags means a future, same-day capture set makes this arm fail and get
            // removed.
            let enabled = |v: &Value| {
                map_get(v, 20)
                    .and_then(|c| map_get(c, 10))
                    .and_then(Value::as_bool)
            };
            assert_eq!((enabled(got), enabled(want)), (Some(false), Some(true)));
            let strip = |v: &Value| {
                let mut v = v.clone();
                if let Some(Value::Map(content)) = map_get_mut(&mut v, 20) {
                    content.retain(|(k, _)| k.as_i64() != Some(10));
                }
                v
            };
            assert!(similar(&strip(got), &strip(want)), "slot 4 beyond the flag");
            continue;
        }
        assert!(
            similar(got, want),
            "slot {i}:\n  got {got:?}\n want {want:?}"
        );
    }
}

/// The looper: the POD Go's `@type` 4 converts through the HX looper encoder, because the
/// device writes the identical slot shape. The oracle is the owner's 2026-09-01 startup capture
/// (a `HD2_LooperMono` dropped into an otherwise stock preset, slot 3, bypassed); the tones are
/// the backup's two looper presets (`SL01[0]`/`SL01[2]`, a `HD2_LooperOneSwitchMono` at slot 1),
/// which were the last two refusals. Different model, different slot, same preset-independent
/// shape: slot kind 7, class 22, the model's `PodGo.sym` index at key 8, and exactly the tone's
/// four stored parameters (`Playback`, `Overdub`, `lowCut`, `highCut`) in a bank at key 7 —
/// not the symbol's ten.
#[test]
fn the_looper_converts_in_the_captured_shape() {
    let (Some(syms), Some(hxb), Some(oracle)) = (
        symbols(),
        backup(),
        PresetStream::parse(
            &std::fs::read(captures().join("looper-preset.msgpack.bin")).unwrap_or_default(),
        )
        .ok(),
    ) else {
        eprintln!("skipping: needs the POD Go looper capture + backup + reference data");
        return;
    };
    let slot =
        |ps: &PresetStream, i: usize| match map_get(&ps.preset, 0).and_then(|m| map_get(m, 22)) {
            Some(Value::Array(slots)) => slots[i].clone(),
            other => panic!("no slot array: {other:?}"),
        };
    // Strip the model index (key 8) and enabled flag (key 10) — the two things that legitimately
    // differ between the captured looper and the backup's — leaving the shape to compare whole.
    let shape = |v: &Value| {
        let mut v = v.clone();
        if let Some(Value::Map(content)) = map_get_mut(&mut v, 20) {
            content.retain(|(k, _)| !matches!(k.as_i64(), Some(8 | 10)));
            content.sort_by_key(|(k, _)| k.as_i64());
        }
        v
    };
    let index_of = |sym: &str| syms.index_of(sym).expect(sym) as i64;

    let captured = slot(&oracle, 3);
    let content = map_get(&captured, 20).expect("looper content");
    assert_eq!(
        map_get(&captured, 19).and_then(Value::as_i64),
        Some(7),
        "slot kind"
    );
    assert_eq!(
        map_get(content, 8).and_then(Value::as_i64),
        Some(index_of("HD2_LooperMono")),
        "the captured looper's model index is its PodGo.sym position (127)"
    );
    assert_eq!(map_get(content, 10).and_then(Value::as_bool), Some(false));

    for idx in [0, 2] {
        let tone = hxb.setlists()[1].presets[idx]
            .clone()
            .unwrap_or_else(|| panic!("SL01[{idx}]"))
            .tone;
        let tone = tone.as_object().expect("tone object").clone();
        let mut donor = oracle.clone();
        vandalise(&mut donor);
        let report = apply_tone(&mut donor, &tone, &syms).expect("the looper preset converts");
        assert!(report.blocks >= 1, "SL01[{idx}] converts its blocks");
        let got = slot(&donor, 1);
        let got_content = map_get(&got, 20).expect("converted looper content");
        assert_eq!(
            map_get(got_content, 8).and_then(Value::as_i64),
            Some(index_of("HD2_LooperOneSwitchMono")),
            "the backup's looper resolves to its own symbol"
        );
        assert_eq!(
            map_get(got_content, 10).and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            shape(&got),
            shape(&captured),
            "SL01[{idx}]: the converted looper must have the captured slot shape"
        );
    }
}
