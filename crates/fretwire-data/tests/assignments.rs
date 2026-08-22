//! The preset's footswitch/controller assignment table (key `4`), read against real captures.
//!
//! The decode this pins was corrected on 2026-08-21 after a live diff on an HX Stomp. Three keys
//! were being read from the wrong place, all three because the *document* numbers an assignment
//! differently from the op-37 *request* that creates one:
//!
//! | field | was read from | is actually |
//! |---|---|---|
//! | parameter index | `6 → 28` (the path — `0` on everything) | `6 → 29` |
//! | travel ends     | keys `4` / `7` (`0` on everything)      | keys `2` / `3` |
//! | entries         | `first()` per source                    | all of them |
//!
//! The parameter-index bug was invisible for as long as it was: reading the path yields `0`, and an
//! assignment onto parameter 0 is common enough that the output looked right. `assign_two_footswitches`
//! exists precisely because it contains one of each — `Time` (index 0, where both readings agree) and
//! `Mix` (index 2, where they do not).
//!
//! Fixtures are our own reassembled captures in captures/ (tracked); no Line 6 data needed.

use std::path::PathBuf;

use fretwire_data::stream::PresetStream;

fn capture(name: &str) -> PresetStream {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../captures")
        .join(name);
    let bytes = std::fs::read(p).expect("read capture fixture");
    PresetStream::parse(&bytes).expect("parse preset stream")
}

/// Captured live: `Time` (parameter 0) on FS1, then `Mix` (parameter 2) on FS2, on one Simple Delay
/// at slot 16. Both were made from the front panel and read back out of the edit buffer.
///
/// This is the regression that matters. Under the old reading both rows reported `param 0`, because
/// key `28` is the model path and is `0` in both.
#[test]
fn reads_the_parameter_index_from_key_29() {
    let a = capture("assign_two_footswitches.msgpack.bin").assignments();
    assert_eq!(a.len(), 2, "two assignments were made on this preset");

    // FS1 -> Simple Delay `Time`, which really is parameter 0.
    assert_eq!(a[0].controller, 3, "FS1 is source ordinal 3");
    assert_eq!(a[0].target_slot, Some(16));
    assert_eq!(a[0].param_index, Some(0));

    // FS2 -> the same block's `Mix`, parameter 2. The old decoder said 0 here.
    assert_eq!(a[1].controller, 4, "FS2 is source ordinal 4");
    assert_eq!(a[1].target_slot, Some(16));
    assert_eq!(a[1].param_index, Some(2));

    // The path sits where the index used to be read from, and is zero on both.
    assert_eq!(a[0].path, Some(0));
    assert_eq!(a[1].path, Some(0));
}

/// The travel ends come from keys `2`/`3`. Keys `4`/`7` are zero on every sample we hold, so the
/// old reading described every assignment as a sweep from 0 to 0.
#[test]
fn reads_the_travel_from_keys_2_and_3() {
    let a = capture("assign_two_footswitches.msgpack.bin").assignments();

    // A delay time, swept 0..8 in its own raw units.
    assert_eq!(a[0].min.as_ref().and_then(rmpv::Value::as_f64), Some(0.0));
    assert_eq!(a[0].max.as_ref().and_then(rmpv::Value::as_f64), Some(8.0));

    // A mix, swept across its full 0..1.
    assert_eq!(a[1].min.as_ref().and_then(rmpv::Value::as_f64), Some(0.0));
    assert_eq!(a[1].max.as_ref().and_then(rmpv::Value::as_f64), Some(1.0));
}

/// The older `dual_amp` capture, which is where the wrong reading was first caught: its one
/// assignment targets `OD Switch`, parameter **9** of a Grammatico GSG at slot 15. The old decoder
/// reported parameter 0 — i.e. `Drive`, a different control entirely.
///
/// Its travel is `false`/`true` rather than numbers, which is why the ends are kept as raw values.
#[test]
fn dual_amp_assignment_targets_parameter_9() {
    let a = capture("dual_amp_stream.msgpack.bin").assignments();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].controller, 7);
    assert_eq!(a[0].target_slot, Some(15));
    assert_eq!(a[0].param_index, Some(9), "OD Switch, not Drive");
    assert_eq!(a[0].path, Some(0));
    assert_eq!(
        a[0].min.as_ref().and_then(rmpv::Value::as_bool),
        Some(false)
    );
    assert_eq!(a[0].max.as_ref().and_then(rmpv::Value::as_bool), Some(true));
}

/// A preset with nothing assigned must decode to nothing, not to ten empty rows.
#[test]
fn an_unassigned_preset_has_no_assignments() {
    for name in [
        "preset1_stream.msgpack.bin",
        "split_preset_stream.msgpack.bin",
    ] {
        assert!(
            capture(name).assignments().is_empty(),
            "{name} has an empty key-4 table"
        );
    }
}

/// Key `1` is **not** parameter-vs-bypass. `tonepush` documents it as "4 a parameter, 0 a bypass";
/// every assignment captured here is a parameter, and two of the three carry `0`.
///
/// What it *is* tracks the target's value type — `0` on the continuous parameters, `4` on the
/// boolean one. That part is a hypothesis with three supporting samples and no counter-example; this
/// test guards the observation so a future capture that breaks it fails loudly.
#[test]
fn key_1_is_not_parameter_versus_bypass() {
    let switched = capture("dual_amp_stream.msgpack.bin").assignments();
    assert_eq!(switched[0].ctype, Some(4), "OD Switch is boolean");
    assert!(switched[0].param_index.is_some(), "and it is a parameter");

    let continuous = capture("assign_two_footswitches.msgpack.bin").assignments();
    assert_eq!(continuous[0].ctype, Some(0), "Time is continuous");
    assert_eq!(continuous[1].ctype, Some(0), "Mix is continuous");
    // Both carry a parameter reference, so key 1 == 0 cannot mean "bypass".
    assert!(continuous.iter().all(|a| a.param_index.is_some()));
}

/// Putting a block's **bypass** on a footswitch does not touch key `4`. It is recorded only in
/// `3 → 8`, the footswitch layout, as a type-1 (DSP block) node.
///
/// Captured by assigning a Simple Delay's bypass to FS1 on a Stomp: key `4` stays entirely `nil`.
/// This is why no key-4 bypass entry has been captured here — on this device that path does not
/// produce one. `tonepush` does show a bypass inside key 4, but its example is a wah auto-engaging
/// off an expression pedal, which is a different feature from a footswitch bypass.
#[test]
fn a_footswitch_bypass_is_not_in_the_assignment_table() {
    let ps = capture("assign_bypass_on_fs1.msgpack.bin");
    assert!(
        ps.assignments().is_empty(),
        "a footswitch bypass must not appear in key 4"
    );
    // It is in the footswitch layout instead, on FS1, naming the block it toggles.
    let fs = ps.footswitch_layout();
    let fs1 = fs[0].as_ref().expect("FS1 is bound");
    assert_eq!(fs1.slot, Some(16), "the Simple Delay");
    assert_eq!(fs1.node_kind, Some(1), "a DSP block, not a controller node");
}
