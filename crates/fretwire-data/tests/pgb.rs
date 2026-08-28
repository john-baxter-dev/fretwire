//! The contributor's POD Go `.pgb` backup (issue #15, 2026-08-27) — the file that supplied the
//! device's `preset_device_id` and setlist geometry, and that forced the container's section
//! table to be decoded (see `src/hxb.rs`). Like every contributor backup it is personal data and
//! deliberately not in git, so everything here skips on a clean clone; the synthetic tests in
//! `hxb.rs` carry the format itself.

use fretwire_data::hxb::Hxb;
use std::path::PathBuf;

/// The backup, whatever it is called — the file name carries its date.
fn backup() -> Option<Hxb> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../captures/pod-go/backup");
    let pgb = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "pgb"))?;
    Hxb::parse(&std::fs::read(pgb).ok()?).ok()
}

#[test]
fn the_pod_go_backup_reads_like_its_pedal() {
    let Some(b) = backup() else {
        eprintln!("skipping: no .pgb backup in captures/pod-go/backup");
        return;
    };
    // The header device id and the L6UMDArchive's `data.device` agree — this pair is what put
    // `preset_device_id: Some(0x210007)` in the device table.
    assert_eq!(b.device_id, 0x0021_0007);
    let umds = b
        .streams
        .iter()
        .find_map(|s| {
            let j: serde_json::Value = serde_json::from_slice(s).ok()?;
            (j.get("schema")?.as_str()? == "L6UMDArchive").then_some(j)
        })
        .expect("the backup carries a L6UMDArchive section");
    assert_eq!(
        umds.get("data")
            .and_then(|d| d.get("device"))
            .and_then(|v| v.as_u64()),
        Some(0x0021_0007)
    );
    // POD Go Edit writes no comment, and the parser must not misread the setlist names as one.
    assert_eq!(b.comment, "");
    // "On the POD Go there are two banks of presets: 'Factory' and 'User'. Each has 128 slots" —
    // the owner's panel description, and the file agrees with it three ways: the SLNM section,
    // each setlist's own name, and the slot counts.
    assert_eq!(b.setlist_names, ["Factory", "User"]);
    let sets = b.setlists();
    assert_eq!(sets.len(), 2);
    for (s, name) in sets.iter().zip(["Factory", "User"]) {
        assert_eq!(s.name, name);
        assert_eq!(s.presets.len(), 128);
    }
    // Factory content is Line 6's and stable: 01A is the preset the owner named.
    assert_eq!(sets[0].presets[0].as_ref().unwrap().name, "US Deluxe Nrm");
}
