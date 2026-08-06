//! `declared_stream_len` reads the preset-stream envelope's self-declared length. The reassembler
//! relies on it to know when a stream is whole instead of guessing from "first short chunk" — the
//! guess truncated Floor reads at 256-byte boundaries (see fretwire-core::session::read_preset).
//!
//! Fixtures are our own reassembled captures in captures/ (tracked); no Line 6 data needed.

use std::path::PathBuf;

use fretwire_data::stream::declared_stream_len;

fn capture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../captures")
        .join(name);
    std::fs::read(p).expect("read capture fixture")
}

/// The declared length must match each fixture's real reassembled size — exactly, or one byte
/// short when the device appended a trailing pad (preset1). It is a *minimum* target the reader
/// keeps reading toward, so it must never exceed the true size.
#[test]
fn declared_length_matches_fixtures() {
    for name in [
        "dual_amp_stream.msgpack.bin",
        "split_preset_stream.msgpack.bin",
        "preset1_stream.msgpack.bin",
    ] {
        let bytes = capture(name);
        let declared =
            declared_stream_len(&bytes).unwrap_or_else(|| panic!("{name}: no declared length"));
        assert!(
            declared <= bytes.len() && bytes.len() - declared <= 1,
            "{name}: declared {declared} not within [len-1, len] of actual {}",
            bytes.len(),
        );
    }
}

/// Exact values, pinned so a change in the envelope layout is caught.
#[test]
fn declared_length_exact_values() {
    assert_eq!(
        declared_stream_len(&capture("dual_amp_stream.msgpack.bin")),
        Some(2609)
    );
    assert_eq!(
        declared_stream_len(&capture("split_preset_stream.msgpack.bin")),
        Some(2857)
    );
    // preset1 carries one trailing pad byte (file is 2804); the declared payload is 2803.
    assert_eq!(
        declared_stream_len(&capture("preset1_stream.msgpack.bin")),
        Some(2803)
    );
}

/// A truncated chunk #0 with the declared length reached mid-payload is exactly the failure the
/// reader must survive: the length says "keep going", so it must be readable from the first chunk.
#[test]
fn declared_length_available_from_first_chunk() {
    // chunk #0 is 256 bytes; the envelope's length field sits well inside it.
    let bytes = capture("preset1_stream.msgpack.bin");
    let chunk0 = &bytes[..256.min(bytes.len())];
    assert_eq!(declared_stream_len(chunk0), Some(2803));
}

#[test]
fn rejects_short_and_implausible() {
    assert_eq!(declared_stream_len(&[]), None);
    assert_eq!(declared_stream_len(&[0, 0, 0, 0, 1]), None); // < 8-byte prefix
    // marker/type = 0, len = 0 -> total would be just the prefix (no payload): rejected.
    assert_eq!(declared_stream_len(&[0, 0, 0, 0, 0, 0, 0, 0]), None);
    // A garbage length (0xFFFF_FFFF) must not be trusted, or the reader loops forever.
    assert_eq!(
        declared_stream_len(&[0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]),
        None
    );
}
