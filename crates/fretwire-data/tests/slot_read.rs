//! Op 4 reads the preset stored in a slot without loading it. Its replies come in two shapes.
//!
//! The ordinary one is a preset document, byte-identical to what select-and-read returns for the
//! same slot — verified live on an HX Stomp, where the two differ only in the envelope's stream-kind
//! marker and the transaction counter.
//!
//! The other is `{102: txn, 103: 0, 104: nil}`: status 0, so the device is answering rather than
//! refusing, but there is no document attached. It has to be told apart from a desynced read,
//! because both fail to parse and only one is worth retrying.
//!
//! Fixtures are our own reassembled captures in captures/ (tracked); no Line 6 data needed.

use std::path::PathBuf;

use fretwire_data::stream::{PresetStream, is_empty_slot_reply};

fn capture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../captures")
        .join(name);
    std::fs::read(p).expect("read capture fixture")
}

/// Captured from bank 0 slot 102 on an HX Stomp: seventeen bytes, and the last one is `nil`.
#[test]
fn recognises_the_empty_slot_reply() {
    let bytes = capture("empty_slot_reply.msgpack.bin");
    assert_eq!(bytes.len(), 17, "envelope plus a three-entry map");
    assert!(is_empty_slot_reply(&bytes));
    // It must not be mistaken for a document, which is the whole reason it needs its own check.
    assert!(PresetStream::parse(&bytes).is_err());
}

/// Real documents must not trip the check, whatever their size — including the empty-*preset*
/// documents that unused slots stream, which are a different thing entirely from an empty *reply*.
#[test]
fn a_real_document_is_not_an_empty_reply() {
    for name in [
        "preset1_stream.msgpack.bin",
        "dual_amp_stream.msgpack.bin",
        "split_preset_stream.msgpack.bin",
        "assign_two_footswitches.msgpack.bin",
    ] {
        let bytes = capture(name);
        assert!(!is_empty_slot_reply(&bytes), "{name} is a document");
        assert!(PresetStream::parse(&bytes).is_ok(), "{name} parses");
    }
}

/// Truncated or empty input is not an empty-slot reply either — it is a failed read, and calling it
/// "the slot is empty" would silently drop a preset from a backup.
#[test]
fn a_truncated_read_is_not_an_empty_reply() {
    assert!(!is_empty_slot_reply(&[]));
    assert!(!is_empty_slot_reply(&[0x00; 8]));
    let mut short = capture("empty_slot_reply.msgpack.bin");
    short.truncate(8); // envelope only, no map
    assert!(!is_empty_slot_reply(&short));
}
