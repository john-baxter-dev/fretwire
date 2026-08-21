//! The HX device table: lookups, and the invariants that keep it honest.
//!
//! Everything asserted here is either a USB ID from a real descriptor or a value read out of a real
//! preset — nothing is inferred from another device in the family. See `docs/helix-floor.md`.

use fretwire_protocol::{DEVICES, Device, PID_HELIX_FLOOR, PID_HX_STOMP, PID_HX_STOMP_XL, Support};

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
    assert!(Device::by_model_code("P99").is_none());
    // An unknown model code must not accidentally match the XL's `None`.
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
fn the_unverified_device_claims_nothing_it_hasnt_shown_us() {
    let xl = Device::by_pid(PID_HX_STOMP_XL).unwrap();
    // Reported, not Verified: an owner has it working, which is real information, but no traffic
    // from one has been reconciled against our builders.
    assert_eq!(xl.support, Support::Reported);
    assert!(
        xl.support.caveat().is_some(),
        "an unverified device says so"
    );
    // We have no capture, preset or backup from an XL — so none of this may be assumed to match
    // the Stomp's just because the name is similar. What an owner *reads off the panel* can fill in
    // (see `the_xl_banks_by_four_not_the_stomps_three`); what only traffic can settle stays empty,
    // and that is the point of the middle tier.
    assert!(xl.model_code.is_none());
    assert!(xl.preset_device_id.is_none());
    assert!(xl.dsps.is_none());
    assert!(xl.snapshots.is_none());
    // …but enumeration still has to do something sane with it.
    assert_eq!(
        xl.dsp_count(),
        1,
        "unknown DSP count falls back to one group"
    );
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
    let floor = Device::by_pid(PID_HELIX_FLOOR).unwrap();
    assert_eq!(floor.presets_per_bank, None);
    assert_eq!(floor.preset_label(0), None);
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
