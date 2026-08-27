//! The key-`4` controller table is **device-sized**, not a fixed ten.
//!
//! It was read as ten entries with footswitches at 3..=7, MIDI at 8 and snapshots at 9. That is an
//! HX Stomp's shape, mistaken for the format's, and it went unchallenged because every capture we
//! had came off a Stomp. An HX Stomp XL owner assigned a Stupor OD's `Drive` to FS6 from the front
//! panel and sent the before/after streams (issue #13, 2026-08-25): the table is **13** entries and
//! FS6 files itself at ordinal **8** — the index we called MIDI.
//!
//! Two later presets off the same XL finished the shape (issue #13, 2026-08-25): one put bypasses on
//! **EXP1/EXP2** and a parameter on **FS8**, the other put a parameter under a **MIDI CC** and
//! another under **Snapshots**. The top two entries had been arithmetic since the table was first
//! read; they are now observed at 11 and 12.
//!
//! What these tests pin:
//!
//! * the table length against the footswitch count, on both devices — the formula
//!   `fretwire_protocol::edit::source::table_len` computes, so the two cannot drift apart
//! * FS6 = 8 on an XL, which is the observation itself, and FS8 = 10 at the run's far end
//! * MIDI = 11 and Snapshots = 12 on an XL, the two that used to be computed
//! * that a *parameter* on a footswitch does not make the block read as bypass-bound to it
//! * that a bypass reaches key `4` from an expression pedal and the footswitch layout from a switch
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
    // Six HX Stomp captures (`P33`) and four HX Stomp XL captures (`P36`).
    for name in [
        "assign_two_footswitches.msgpack.bin",
        "assign_bypass_on_fs1.msgpack.bin",
        "assign_bypass_and_param.msgpack.bin",
        "preset1_stream.msgpack.bin",
        "dual_amp_stream.msgpack.bin",
        "split_preset_stream.msgpack.bin",
        "xl_no_assignments.msgpack.bin",
        "xl_assign_param_fs6.msgpack.bin",
        "xl_exp_bypass.msgpack.bin",
        "xl_assign_midi_and_snapshots.msgpack.bin",
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

    // HX Stomp XL: 3..=10, then 11 and 12 — every one of them read off the device by now, the top
    // two by `an_xl_files_midi_at_eleven_and_snapshots_at_twelve` below.
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

/// The two entries above the footswitch run, read off a device at last.
///
/// They were the last of this table still computed: the length formula puts MIDI and snapshots
/// immediately above the switches, and on an XL that is 11 and 12, but every sample we held stopped
/// at the run. An owner put a Teemah's `Gain` under `CC5` and a Stupor OD's `Drive` under Snapshots
/// in one preset, which lands both in the same capture.
#[test]
fn an_xl_files_midi_at_eleven_and_snapshots_at_twelve() {
    let ps = capture("xl_assign_midi_and_snapshots.msgpack.bin");
    assert_eq!(ps.device_model().as_deref(), Some("P36"));
    assert_eq!(ps.footswitch_layout().len(), 8);
    assert_eq!(ps.controller_table_len(), Some(13));

    let a = ps.assignments();
    assert_eq!(
        a.len(),
        2,
        "one MIDI assignment and one snapshot assignment"
    );

    let midi = a.iter().find(|x| x.controller == 11).expect("MIDI at 11");
    let snap = a
        .iter()
        .find(|x| x.controller == 12)
        .expect("Snapshots at 12");

    // The ordinals the formula computes, now with a reading behind them.
    assert_eq!(source::midi(8), 11);
    assert_eq!(source::snapshots(8), 12);
    assert_eq!(source::name(11, 8), "MIDI");
    assert_eq!(source::name(12, 8), "Snapshots");

    // Both drive a parameter, so both carry a parameter reference.
    assert_eq!(midi.param_index, Some(0), "the Teemah's Gain");
    assert_eq!(snap.param_index, Some(0), "the Stupor OD's Drive");

    // Inner key `1` is the CC number under a MIDI source. The snapshot entry drives an equally
    // continuous parameter and carries 0, which is what rules out reading this as a value type.
    assert_eq!(midi.ctype, Some(5), "CC5, as the owner set it");
    assert_eq!(snap.ctype, Some(0));
}

/// A bypass goes to key `4` from an expression pedal and to the footswitch layout from a switch —
/// in one preset, so the split is visible in a single document rather than argued across two.
#[test]
fn a_bypass_picks_its_table_by_the_source_that_drives_it() {
    let ps = capture("xl_exp_bypass.msgpack.bin");
    assert_eq!(ps.controller_table_len(), Some(13));

    let a = ps.assignments();

    // EXP1 and EXP2 carry bypasses: a target slot and no parameter reference.
    let exp: Vec<_> = a.iter().filter(|x| x.controller <= 2).collect();
    assert_eq!(exp.len(), 3, "two on EXP1, one on EXP2");
    for e in &exp {
        assert_eq!(e.param_index, None, "a bypass names no parameter");
        assert!(e.target_slot.is_some(), "but it does name a slot");
    }
    assert_eq!(source::name(1, 8), "EXP1");
    assert_eq!(source::name(2, 8), "EXP2");

    // FS8 in the same preset carries a *parameter*, at the far end of the run.
    let fs8 = a.iter().find(|x| x.controller == 10).expect("FS8 at 10");
    assert_eq!(source::footswitch(8, 8), Some(10));
    assert_eq!(fs8.param_index, Some(2), "the Dhyana Drive's Tone");

    // And the FS7 *bypass* is not here at all — it is a type-1 node in the footswitch layout.
    assert!(
        !a.iter().any(|x| x.controller == 9),
        "FS7 would be ordinal 9, and a bypass on a switch never reaches this table"
    );
    let layout = ps.footswitch_layout();
    let fs7 = layout[6].as_ref().expect("FS7 carries the Teemah's bypass");
    assert_eq!(fs7.node_kind, Some(1), "a DSP block, not a controller node");
}

/// One source can drive several things, and a target slot need not still hold a block.
///
/// EXP1 in this capture has two entries, one of them pointing at slot 1 — which is empty, the
/// preset's blocks starting at slot 2. The owner rebuilt this preset by hand, so the likeliest
/// story is an assignment left behind by a block that moved; what matters here is that the document
/// contains one and nothing downstream may assume the lookup succeeds. `AssignmentDto` resolves the
/// parameter name through `find(|b| b.slot == slot)?`, so an orphan reads as an unnamed row rather
/// than a panic.
#[test]
fn a_source_may_hold_several_entries_and_one_may_be_orphaned() {
    let ps = capture("xl_exp_bypass.msgpack.bin");
    let a = ps.assignments();

    let exp1: Vec<_> = a.iter().filter(|x| x.controller == 1).collect();
    assert_eq!(
        exp1.len(),
        2,
        "key 4 holds an array per source, not one entry"
    );

    let occupied: Vec<i64> = ps.loaded_blocks().iter().map(|b| b.slot).collect();
    assert!(
        !occupied.contains(&1),
        "slot 1 holds no block in this preset"
    );
    assert!(
        exp1.iter().any(|e| e.target_slot == Some(1)),
        "and one EXP1 entry points at it anyway"
    );
}
