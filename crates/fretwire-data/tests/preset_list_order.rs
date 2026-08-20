//! The browse listing is **already in slot order**: a row's position is the preset's slot, and the
//! map key it carries is not (see [`fretwire_data::stream::PresetListEntry`]).
//!
//! Checked against the only real traffic we have from a device with reordered presets: HX Edit's
//! own listing of a contributor's Helix Floor, with that same unit's `.hxb` backup as ground
//! truth. Three presets on it have been moved, so sorting by the key — which is what
//! `list_presets_in` used to do — puts 36 of bank 0's rows and 12 of bank 1's under the wrong
//! number. This is the test that would have caught it.
//!
//! **Both inputs are a contributor's own presets and are not in git** (`captures/helix-floor/` is
//! ignored wholesale). The test skips when they are absent, so a clean clone stays green. To
//! regenerate the listings from the capture:
//!
//! ```text
//! tools/extract-preset-stream.py captures/helix-floor/WinCap5.pcapng \
//!     captures/helix-floor/floor-list --list
//! ```
//!
//! which writes three streams: `floor-list0` is bank 1, `floor-list2` is bank 0 (`floor-list1` is
//! a different array-valued resource and is not a listing). Rename them to the two paths below.

use std::path::PathBuf;

use fretwire_data::{hxb::Hxb, stream::parse_preset_list};

fn captures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/helix-floor")
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

/// One bank's listing, against that bank in the backup.
///
/// Every row must name the preset the backup holds at that **position**. Bank 0 of this unit is the
/// interesting one: `InSTANtgH0St/24` sits at position 101 carrying key 68, and `Parallel Muffs` at
/// 108 carrying key 107, so a position-for-position match is only possible if the key is ignored.
fn check_bank(file: &str, bank: usize) {
    let path = captures().join(file);
    let (Ok(raw), Some(hxb)) = (std::fs::read(&path), backup()) else {
        eprintln!(
            "skipping: {} or the .hxb backup is not present",
            path.display()
        );
        return;
    };
    let listing = parse_preset_list(&raw).expect("the listing parses");
    let setlist = &hxb.setlists()[bank];
    assert_eq!(listing.len(), setlist.presets.len(), "one row per slot");

    for row in &listing {
        // An empty backup slot has no name to compare against; the device still lists it.
        let Some(stored) = &setlist.presets[row.slot as usize] else {
            continue;
        };
        assert_eq!(
            (row.slot, &row.name),
            (stored.index as u16, &stored.name),
            "{}: the row at position {} is not the preset the backup holds there",
            setlist.name,
            row.slot,
        );
    }

    // And the fixture has to actually exercise a reorder, or the assertion above passes for the
    // wrong reason — on an untouched device the two orders are identical and prove nothing.
    let base = (bank * setlist.presets.len()) as i64;
    let moved = listing.iter().filter(|r| r.key_disagrees(base)).count();
    assert!(
        moved > 0,
        "{}: no row carries a key of its own, so this fixture can't tell the two orders apart",
        setlist.name,
    );

    let mut by_key = listing.clone();
    by_key.sort_by_key(|r| r.key);
    assert_ne!(
        by_key.iter().map(|r| &r.name).collect::<Vec<_>>(),
        listing.iter().map(|r| &r.name).collect::<Vec<_>>(),
        "{}: sorting by key must change the order — that is the bug this pins",
        setlist.name,
    );
    eprintln!(
        "{}: {} rows, {moved} carrying a key other than their slot, all in slot order",
        setlist.name,
        listing.len(),
    );
}

#[test]
fn factory_1_lists_in_slot_order_despite_two_moved_presets() {
    check_bank("floor-bank0-list.msgpack.bin", 0);
}

#[test]
fn factory_2_lists_in_slot_order_despite_a_moved_preset() {
    check_bank("floor-bank1-list.msgpack.bin", 1);
}
