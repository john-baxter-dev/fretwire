//! A POD Go Volume and Wah can carry `0x44000001` in the paired-model key `24 → 26` where every
//! other block without a cab stores `-1` (the looper preset off the original owner's pedal, and a
//! second owner's preset in issue #15, 2026-10-14). That value indexes nothing, and reading it as a
//! cab printed "+ cab: <unknown>" under both blocks. It must read as "no paired model".
//!
//! Personal device data: the capture stays out of git, so this skips on a clean clone.

use fretwire_data::stream::PresetStream;
use std::path::PathBuf;

#[test]
fn an_out_of_range_paired_index_is_no_cab() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../captures/pod-go/looper-preset.msgpack.bin");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipping: {} not present", path.display());
        return;
    };
    let blocks = PresetStream::parse(&bytes).unwrap().effect_blocks();
    // Slot 1 is the Volume, slot 2 the Wah: both hold 0x44000001 on the wire.
    for b in blocks.iter().take(2) {
        assert_eq!(
            b.paired_ref, None,
            "slot {} model {:?}",
            b.index, b.model_ref
        );
        assert!(b.paired_params.is_empty());
    }
    assert!(blocks.iter().all(|b| b.paired_ref.is_none()));
}
