//! Backup file format over a real captured stream: a `Backup` built from the raw read-stream
//! payload must JSON-round-trip byte-exactly, and the stored raw must be restore-ready
//! (`PresetStream::parse` → `to_blob`, exactly what `Session::restore_preset` replays).
//!
//! Uses a captured preset fixture from the in-repo `captures/` dir (our own device recording), so
//! it needs no imported reference data and runs on any checkout.

use fretwire_core::backup::{Backup, BackupPreset};
use fretwire_data::stream::PresetStream;
use std::path::PathBuf;

fn data(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../captures")
        .join(name);
    std::fs::read(p).unwrap()
}

#[test]
fn real_stream_round_trips_and_is_restore_ready() {
    let raw = data("preset1_stream.msgpack.bin");
    let backup = Backup {
        device: "HX Stomp".into(),
        presets: vec![BackupPreset {
            index: 3,
            name: "DIRTY MADS".into(),
            raw: raw.clone(),
        }],
    };

    let restored = Backup::from_json(&backup.to_json()).unwrap();
    assert_eq!(restored, backup);

    // The stored raw is exactly what restore replays: parse → to_blob must succeed and yield a
    // non-trivial op-21 payload.
    let entry = restored.preset(3).unwrap();
    assert_eq!(entry.raw, raw);
    let ps = PresetStream::parse(&entry.raw).unwrap();
    let blob = ps.to_blob();
    assert!(
        blob.len() > 1000,
        "implausibly small preset blob: {} bytes",
        blob.len()
    );
}
