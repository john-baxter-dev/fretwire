//! Validate the frame codec against **real full frames** captured from HX Edit (header included),
//! and pin down what the offset-12 `arg` field is: a per-channel running stream offset — NOT a
//! per-packet checksum. (Bytes from open_two_presets_one_after_another.pcapng via tshark.)

use fretwire_protocol::Frame;

fn hx(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

// Idle keepalives (cmd 0x10) — body empty.
const IDLE_F003_299: &str = "080000180210f003005200109c1b0000";
const IDLE_ED03_585: &str = "080000188010ed0300620010d9230000";
const IDLE_ED03_2095: &str = "080000188010ed0300630010d9230000";
// Data frames on the edit channel (open + paged chunk reads).
const OPEN_2321: &str =
    "1d0000188010ed0300640004d9230000010006000d0000008366cd043a641465826b006c13000000";
const STREAM_2419: &str =
    "190000188010ed030067000c1a24000001000600090000008366cd043c641665c0000000";
const CHUNK_2427: &str = "080000188010ed03006800081a250000";

#[test]
fn decodes_real_frame_fields() {
    let f = Frame::decode(&hx(OPEN_2321)).unwrap();
    assert_eq!(f.magic, 0x18);
    assert_eq!(f.src, 0x1080); // 8010 LE
    assert_eq!(f.dst, 0x03ed); // ed03 LE
    assert_eq!(f.seq, 0x64);
    assert_eq!(f.cmd, 0x04); // OPEN
    assert_eq!(f.arg, 0x000023d9);
    // Body is the TLV (opcode 0x0006, 13-byte msgpack edit/open body).
    assert_eq!(&f.body[0..4], &[0x01, 0x00, 0x06, 0x00]);
}

#[test]
fn all_real_frames_round_trip_byte_exact() {
    for s in [
        IDLE_F003_299,
        IDLE_ED03_585,
        OPEN_2321,
        STREAM_2419,
        CHUNK_2427,
    ] {
        let raw = hx(s);
        assert_eq!(
            Frame::decode(&raw).unwrap().encode(),
            raw,
            "round-trip failed for {s}"
        );
    }
}

#[test]
fn arg_is_not_a_checksum_but_a_per_channel_offset() {
    // Two idle frames on ed03 with identical content carry the SAME arg -> not a content checksum.
    let a = Frame::decode(&hx(IDLE_ED03_585)).unwrap();
    let b = Frame::decode(&hx(IDLE_ED03_2095)).unwrap();
    assert_eq!(a.arg, b.arg);
    assert_eq!(a.arg, 0x000023d9);
    // Different channels have different base args.
    assert_eq!(Frame::decode(&hx(IDLE_F003_299)).unwrap().arg, 0x00001b9c);

    // On the paged preset stream, arg advances by the 256-byte chunk size between reads.
    let s = Frame::decode(&hx(STREAM_2419)).unwrap().arg; // 0x241a
    let c = Frame::decode(&hx(CHUNK_2427)).unwrap().arg; // 0x251a
    assert_eq!(c - s, 0x100); // +256 = one chunk
}
