//! The key-`4` controller table is **device-sized**, not a fixed ten.
//!
//! It was read as ten entries with footswitches at 3..=7, MIDI at 8 and snapshots at 9. That is an
//! HX Stomp's shape, mistaken for the format's, and it held for a year because every capture we
//! had came off a Stomp. An HX Stomp XL owner assigned a Stupor OD's `Drive` to FS6 from the front
//! panel and sent the before/after streams (issue #13, 2026-08-25): the table is **13** entries and
//! FS6 files itself at ordinal **8** — the index we called MIDI.
//!
//! What these tests pin:
//!
//! * the table length against the footswitch count, on both devices — the formula
//!   `fretwire_protocol::edit::source::table_len` computes, so the two cannot drift apart
//! * FS6 = 8 on an XL, which is the observation itself
//! * that a *parameter* on a footswitch does not make the block read as bypass-bound to it
//!
//! Fixtures are reassembled captures in `captures/` (tracked); no Line 6 data needed.

use fretwire_data::stream::PresetStream;
use fretwire_protocol::edit::source;

fn capture(name: &str) -> PresetStream {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../captures")
        .join(name);
    PresetStream::parse(&std::fs::read(p).expect("read capture fixture")).expect("parse stream")
}

/// Every stream we hold, across two devices. `table == footswitches + 5` on all of them, which is
/// the whole basis for computing the ordinals rather than hard-coding them.
#[test]
fn the_table_is_five_longer_than_the_footswitch_count() {
    // Six HX Stomp captures (`P33`) and two HX Stomp XL captures (`P36`).
    for name in [
        "assign_two_footswitches.msgpack.bin",
        "assign_bypass_on_fs1.msgpack.bin",
        "assign_bypass_and_param.msgpack.bin",
        "preset1_stream.msgpack.bin",
        "dual_amp_stream.msgpack.bin",
        "split_preset_stream.msgpack.bin",
        "xl_no_assignments.msgpack.bin",
        "xl_assign_param_fs6.msgpack.bin",
    ] {
        let ps = capture(name);
        let switches = ps.footswitch_layout().len();
        assert_eq!(
            ps.controller_table_len(),
            Some(source::table_len(switches)),
            "{name}: {switches} switches, so the table should be {}",
            source::table_len(switches),
        );
    }
}

/// The two shapes, stated outright so a regression names the device rather than an arithmetic slip.
#[test]
fn a_stomp_holds_ten_entries_and_an_xl_thirteen() {
    let stomp = capture("preset1_stream.msgpack.bin");
    assert_eq!(stomp.device_model().as_deref(), Some("P33"));
    assert_eq!(stomp.footswitch_layout().len(), 5);
    assert_eq!(stomp.controller_table_len(), Some(10));

    let xl = capture("xl_no_assignments.msgpack.bin");
    assert_eq!(xl.device_model().as_deref(), Some("P36"));
    assert_eq!(xl.footswitch_layout().len(), 8);
    assert_eq!(xl.controller_table_len(), Some(13));

    // An empty table is still full-length: the size is the device's, not a count of what is in it.
    assert!(xl.assignments().is_empty());
}

/// The observation that killed the constant. On a Stomp, ordinal 8 is MIDI.
#[test]
fn an_xls_sixth_footswitch_takes_the_ordinal_a_stomp_calls_midi() {
    let a = capture("xl_assign_param_fs6.msgpack.bin").assignments();
    assert_eq!(a.len(), 1, "one assignment was made on this preset");

    assert_eq!(a[0].controller, 8, "FS6 on an eight-switch device");
    assert_eq!(
        source::footswitch(6, 8),
        Some(8),
        "and that is what we compute"
    );
    assert_eq!(source::midi(5), 8, "which is MIDI on a five-switch device");

    // Stupor OD at slot 1, `Drive` = parameter 0, swept across its full range.
    assert_eq!(a[0].target_slot, Some(1));
    assert_eq!(a[0].param_index, Some(0));
    assert_eq!(a[0].min.as_ref().and_then(rmpv::Value::as_f64), Some(0.0));
    assert_eq!(a[0].max.as_ref().and_then(rmpv::Value::as_f64), Some(1.0));
}

/// A footswitch only carries switches it has, and the ordinals above the run move with the count.
#[test]
fn the_ordinals_above_the_run_move_with_the_device() {
    // HX Stomp: 3..=7, then 8 and 9 — the numbers the old constants held.
    assert_eq!(source::footswitch(1, 5), Some(3));
    assert_eq!(source::footswitch(5, 5), Some(7));
    assert_eq!(
        source::footswitch(6, 5),
        None,
        "a Stomp has no sixth switch"
    );
    assert_eq!(source::midi(5), 8);
    assert_eq!(source::snapshots(5), 9);

    // HX Stomp XL: 3..=10, then 11 and 12. Only the footswitch run is observed; the top two are
    // where the length puts them and nobody has read either back.
    assert_eq!(source::footswitch(1, 8), Some(3));
    assert_eq!(source::footswitch(8, 8), Some(10));
    assert_eq!(source::footswitch(9, 8), None);
    assert_eq!(source::midi(8), 11);
    assert_eq!(source::snapshots(8), 12);

    // Snapshots is always the last entry, whatever the device.
    for n in [5, 8] {
        assert_eq!(source::snapshots(n) as usize, source::table_len(n) - 1);
    }
}

/// A *parameter* under a footswitch puts a type-2 controller node in the layout pointing at the
/// block's slot. That must not read as the block's bypass being on that switch — they are different
/// mechanisms written by different opcodes, and conflating them would show a phantom `FS6` badge.
#[test]
fn a_parameter_on_a_switch_is_not_a_bypass_on_it() {
    let ps = capture("xl_assign_param_fs6.msgpack.bin");

    let layout = ps.footswitch_layout();
    let fs6 = layout[5].as_ref().expect("FS6 carries the controller node");
    assert_eq!(fs6.node_kind, Some(2), "controller node, not a DSP block");
    assert_eq!(fs6.slot, Some(1), "pointing at the Stupor OD");

    let blocks = ps.loaded_blocks();
    let od = blocks.iter().find(|b| b.slot == 1).expect("the Stupor OD");
    assert_eq!(od.footswitch, 0, "its bypass is on no switch");
}
