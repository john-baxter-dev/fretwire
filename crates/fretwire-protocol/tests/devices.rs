//! The HX device table: lookups, and the invariants that keep it honest.
//!
//! Everything asserted here is either a USB ID from a real descriptor or a value read out of a real
//! preset — nothing is inferred from another device in the family. See `docs/helix-floor.md`.

use fretwire_protocol::{
    DEVICES, Device, PID_HELIX_FLOOR, PID_HELIX_LT, PID_HX_EFFECTS, PID_HX_STOMP, PID_HX_STOMP_XL,
    Support,
};

#[test]
fn every_device_has_a_distinct_pid() {
    let mut pids: Vec<u16> = DEVICES.iter().map(|d| d.pid).collect();
    let before = pids.len();
    pids.sort_unstable();
    pids.dedup();
    assert_eq!(pids.len(), before, "two devices share a PID");
}

#[test]
fn lookup_by_pid() {
    assert_eq!(
        Device::by_pid(PID_HX_STOMP).map(|d| d.name),
        Some("HX Stomp")
    );
    assert_eq!(
        Device::by_pid(PID_HELIX_FLOOR).map(|d| d.name),
        Some("Helix Floor")
    );
    assert_eq!(
        Device::by_pid(PID_HX_STOMP_XL).map(|d| d.name),
        Some("HX Stomp XL")
    );
    assert_eq!(
        Device::by_pid(PID_HELIX_LT).map(|d| d.name),
        Some("Helix LT")
    );
    assert_eq!(
        Device::by_pid(PID_HX_EFFECTS).map(|d| d.name),
        Some("HX Effects")
    );
    assert!(Device::by_pid(0xFFFF).is_none());
}

#[test]
fn lookup_by_preset_model_code() {
    // The code each device stamps into a preset at key `7 → 36`, read from real streams.
    assert_eq!(
        Device::by_model_code("P33").map(|d| d.pid),
        Some(PID_HX_STOMP)
    );
    assert_eq!(
        Device::by_model_code("P21").map(|d| d.pid),
        Some(PID_HELIX_FLOOR)
    );
    // The XL's, from an owner's handshake reply and a preset read off the same unit.
    assert_eq!(
        Device::by_model_code("P36").map(|d| d.pid),
        Some(PID_HX_STOMP_XL)
    );
    assert!(Device::by_model_code("P99").is_none());
    assert!(Device::by_model_code("").is_none());
}

#[test]
fn verified_devices_are_fully_described() {
    for d in DEVICES.iter().filter(|d| d.support == Support::Verified) {
        assert!(
            d.model_code.is_some(),
            "{} is verified but has no model code",
            d.name
        );
        assert!(
            d.preset_device_id.is_some(),
            "{} is verified but has no device id",
            d.name
        );
        assert!(
            d.dsps.is_some(),
            "{} is verified but has no DSP count",
            d.name
        );
        assert!(
            d.snapshots.is_some(),
            "{} is verified but has no snapshot count",
            d.name
        );
    }
}

#[test]
fn the_stomp_xl_claims_nothing_it_hasnt_shown_us() {
    let xl = Device::by_pid(PID_HX_STOMP_XL).unwrap();
    // Still Reported, not Verified — and the reason moved. It used to be "no traffic from one has
    // been reconciled"; since 2026-08-25 we hold two preset streams off an XL (issue #13). Those
    // are *reads*: decoded and reconciled, so what they show is now filled in below. `Verified`
    // means the **builders** have been checked byte-for-byte against the device, and no edit has
    // ever been sent to an XL.
    assert_eq!(xl.support, Support::Reported);
    assert!(
        xl.support.caveat().is_some(),
        "an unverified device says so"
    );
    assert_eq!(xl.model_code, Some("P36"));
    // Read straight out of those streams: key `1` (the second DSP group) is nil, and key `10 → 10`
    // holds SNAPSHOT 1..4. See `fretwire-core/tests/controller_table.rs`, which pins the same
    // fixtures for the controller table's size.
    assert_eq!(xl.dsps, Some(1));
    assert_eq!(xl.snapshots, Some(4));
    assert_eq!(xl.dsp_count(), 1);
    // What the streams do *not* carry stays empty. This is a `.hxb` backup-header field, and no
    // backup from an XL has ever been opened — a preset stream is a different document.
    assert!(xl.preset_device_id.is_none());
    // Likewise the setlist arity: one setlist's size was read off the panel, but whether an XL has
    // several is unobserved, so the names stay absent rather than inheriting the Floor's.
    assert!(xl.setlists.is_none());
    assert_eq!(xl.setlist_size, Some(128));
}

/// The HX Effects is in the table on one `lsusb` line (issue #10) and one owner's word that it
/// works — enough to find one, open it and warn, and not enough to say anything else. A report
/// with no capture behind it moves `support` and nothing else: nothing is inherited from the
/// Stomp, because it is an effects-only unit, so a copied model code or snapshot count would be a
/// guess about a different data class.
#[test]
fn the_hx_effects_carries_no_geometry_it_has_not_been_shown() {
    let fx = Device::by_pid(PID_HX_EFFECTS).unwrap();
    assert_eq!(fx.support, Support::Reported);
    assert!(fx.support.caveat().is_some());
    assert!(fx.model_code.is_none());
    assert!(fx.preset_device_id.is_none());
    assert!(fx.dsps.is_none());
    assert!(fx.snapshots.is_none());
    assert!(fx.setlists.is_none());
    assert!(fx.setlist_size.is_none());
    assert!(fx.presets_per_bank.is_none());
    // The fallbacks still have to do something sane with an all-unknown device.
    assert_eq!(fx.dsp_count(), 1);
    assert_eq!(fx.setlist_stride(), 128);
    assert_eq!(fx.setlist_names(), &["Presets"]);
    assert_eq!(fx.preset_label(0), None);
}

#[test]
fn verified_devices_come_first_so_open_prefers_them() {
    // `Transport::open` takes the first present device in this order. Anything short of Verified
    // sorts after — `Reported` is better than `Untested` but still not a device we would rather
    // open than one whose traffic we have reconciled.
    let first_unverified = DEVICES
        .iter()
        .position(|d| d.support != Support::Verified)
        .unwrap_or(DEVICES.len());
    let last_verified = DEVICES
        .iter()
        .rposition(|d| d.support == Support::Verified)
        .unwrap_or(0);
    assert!(
        last_verified < first_unverified,
        "verified devices must be listed before unverified ones"
    );
}

/// Only `Verified` is silent; everything else tells the user what it is.
#[test]
fn every_support_tier_but_verified_carries_a_caveat() {
    assert_eq!(Support::Verified.caveat(), None);
    assert!(Support::Reported.caveat().is_some());
    assert!(Support::Untested.caveat().is_some());
}

/// The shipped udev rule is what lets a non-root user claim the interface, so a device in the
/// table with no rule is a device that silently fails with EACCES on a normal Linux desktop.
#[test]
fn every_device_has_a_udev_rule() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/70-hxstomp.rules");
    let rules = std::fs::read_to_string(&path).expect("read the packaged udev rules");
    for d in DEVICES {
        let needle = format!("ATTR{{idProduct}}==\"{:04x}\"", d.pid);
        assert!(
            rules.contains(&needle),
            "no udev rule for {} ({needle})",
            d.name
        );
    }
}

#[test]
fn the_two_verified_devices_differ_where_we_measured_them() {
    let stomp = Device::by_pid(PID_HX_STOMP).unwrap();
    let floor = Device::by_pid(PID_HELIX_FLOOR).unwrap();

    assert_eq!((stomp.dsps, stomp.snapshots), (Some(1), Some(3)));
    assert_eq!((floor.dsps, floor.snapshots), (Some(2), Some(8)));
    assert_eq!(stomp.preset_device_id, Some(0x0021_0006));
    assert_eq!(floor.preset_device_id, Some(0x0021_0001));
    assert_ne!(stomp.model_code, floor.model_code);
}

/// The pedal's own preset numbering: `01A`, `01B`, `01C`, `02A`, … A label, never an address.
#[test]
fn preset_labels_read_the_way_the_stomps_screen_does() {
    let stomp = Device::by_pid(PID_HX_STOMP).unwrap();
    let label = |slot| stomp.preset_label(slot).unwrap();

    assert_eq!(label(0), "01A");
    assert_eq!(label(1), "01B");
    assert_eq!(label(2), "01C");
    assert_eq!(label(3), "02A");
    // The preset the live tests sit on, and the last slot of the 126.
    assert_eq!(label(24), "09A");
    assert_eq!(label(125), "42C");
    // 126 slots divide into whole banks — a leftover would mean the table's 3 is wrong.
    assert_eq!(
        stomp.setlist_size.unwrap() % stomp.presets_per_bank.unwrap(),
        0
    );
}

/// The XL banks by **four**, not the Stomp's three — reported off the pedal's own screen, and the
/// one field an owner's reading can fill in that a capture would be needed for otherwise.
#[test]
fn the_xl_banks_by_four_not_the_stomps_three() {
    let xl = Device::by_pid(PID_HX_STOMP_XL).unwrap();
    let label = |slot| xl.preset_label(slot).unwrap();

    assert_eq!(label(0), "01A");
    assert_eq!(label(3), "01D");
    assert_eq!(label(4), "02A");
    // The top of the range the owner reads on the panel: 32 banks of 4.
    assert_eq!(label(127), "32D");
    assert_eq!(
        xl.setlist_size.unwrap() % xl.presets_per_bank.unwrap(),
        0,
        "128 must divide into whole banks of 4"
    );
    // Same slot, different label from the Stomp — which is the reason this is per-device data and
    // not one shared constant.
    let stomp = Device::by_pid(PID_HX_STOMP).unwrap();
    assert_ne!(stomp.preset_label(3), xl.preset_label(3));
}

/// A device whose screen we have not seen gets no label rather than a plausible wrong one — the
/// editor falls back to the slot number. See `Device::presets_per_bank`.
///
/// The Floor is the interesting one: it almost certainly banks by four like the XL, but "almost
/// certainly" is what this table refuses to record. Its owner reading `01A`-`32D` off the panel is
/// all it would take.
#[test]
fn devices_we_have_not_seen_banking_for_offer_no_label() {
    // The Floor and the LT both: the survey read the LT's presets and browsed its setlists but
    // never says how its screen groups them, and the Floor's is unknown too, so neither has
    // anything to inherit.
    for pid in [PID_HELIX_FLOOR, PID_HELIX_LT] {
        let d = Device::by_pid(pid).unwrap();
        assert_eq!(d.presets_per_bank, None, "{}", d.name);
        assert_eq!(d.preset_label(0), None, "{}", d.name);
    }
    // Belt and braces: no device may carry a bank size without the label agreeing, in either
    // direction. This is the invariant the loop above used to be here to protect.
    for d in DEVICES {
        assert_eq!(
            d.presets_per_bank.is_some(),
            d.preset_label(0).is_some(),
            "{} disagrees about whether it can label a preset",
            d.name
        );
    }
}

/// The LT reports the Floor's model code and the same setlist geometry, but its own preset
/// device id was never observed — it must stay unknown rather than inherit the Floor's.
#[test]
fn the_lt_shares_the_floors_data_class_without_inheriting_its_device_id() {
    let floor = Device::by_pid(PID_HELIX_FLOOR).unwrap();
    let lt = Device::by_pid(PID_HELIX_LT).unwrap();

    // Measured on a physical LT: see docs/helix-lt.md.
    assert_eq!(lt.model_code, Some("P21"));
    assert_eq!((lt.dsps, lt.snapshots), (Some(2), Some(8)));
    assert_eq!(lt.setlist_size, floor.setlist_size);
    assert_eq!(lt.setlists, floor.setlists);

    // Never seen on the wire, so never guessed.
    assert_eq!(lt.preset_device_id, None);

    // Both stamp "P21"; the lookup keeps resolving it to the Floor, which is listed first.
    assert_eq!(
        Device::by_model_code("P21").map(|d| d.pid),
        Some(PID_HELIX_FLOOR)
    );
}

/// Setting 27's menu text spells out the pedal's preset range, so it differs per device and
/// `settings::SETTINGS` — one flat table — cannot hold both. These derive from counts that were
/// each read off a screen, which is the point: one set of facts, not two.
#[test]
fn preset_numbering_labels_follow_each_units_own_counts() {
    let stomp = Device::by_pid(PID_HX_STOMP).unwrap();
    let xl = Device::by_pid(PID_HX_STOMP_XL).unwrap();

    // 126 presets, 42 banks of three. Both forms read off the pedal [2026-08-24] — the banked one
    // after this function had already computed it, which is what makes it a check and not a guess.
    assert_eq!(
        stomp.preset_numbering_labels(),
        Some(("000-125".into(), "01A-42C".into()))
    );
    // 128 presets, 32 banks of four. `01A`-`32D` is the owner's own reading [2026-08-21].
    assert_eq!(
        xl.preset_numbering_labels(),
        Some(("000-127".into(), "01A-32D".into()))
    );
    // The two must not agree, or there was no reason to derive them.
    assert_ne!(
        stomp.preset_numbering_labels(),
        xl.preset_numbering_labels()
    );
}

/// The same rule `presets_per_bank` states: an unmeasured bank size produces no label at all, not a
/// plausible one. The Floor and the LT both hold 128 slots and neither has had its screen read, and
/// 128 divides by 4 and by 8 with no evidence either way.
#[test]
fn a_device_with_no_measured_bank_size_offers_no_labels() {
    for pid in [PID_HELIX_FLOOR, PID_HELIX_LT, PID_HX_EFFECTS] {
        let d = Device::by_pid(pid).unwrap();
        assert_eq!(d.preset_numbering_labels(), None, "{}", d.name);
    }
}
