//! Golden tests against real bytes captured from an HX Stomp (see `captures/`, `docs/`).
//! Each frame is decoded, checked field-by-field, then re-encoded and required to reproduce
//! the original wire bytes exactly — proving the codec matches HX Edit's output.

use fretwire_protocol::{channel, cmd, op, Frame, Tlv, MAGIC, MAGIC_HANDSHAKE};

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

// --- real captured frames (full wire bytes incl. header + padding) ---

// capture1 frame 2259: an idle keepalive on the edit channel.
const IDLE: &str = "080000188010ed03002600103a1b0000";
// startup frame 2454 — byte-identical to the reference HANDSHAKE Packet 1.
const HANDSHAKE: &str = "0c0000280110ef03000000020001002100100000";
// reference SESSION_OPEN_1 (Packet 2).
const SESSION_OPEN_1: &str = "110000180110ef030002000400100000010002000100000002000000";
// capture1 frame 2307 — bypass toggle (op 0x0006, ilen 13) on the edit channel.
const BYPASS: &str = "1d0000188010ed03002700043a1b0000010006000d0000008366cd03f16429658262073bc2000000";

fn round_trip(name: &str, wire_hex: &str) -> Frame {
    let wire = hex(wire_hex);
    let f = Frame::decode(&wire).unwrap_or_else(|e| panic!("decode {name}: {e}"));
    assert_eq!(f.encode(), wire, "{name} did not re-encode to the original bytes");
    f
}

#[test]
fn idle_frame() {
    let f = round_trip("IDLE", IDLE);
    assert_eq!(f.magic, MAGIC);
    assert_eq!((f.src, f.dst), channel::EDIT);
    assert_eq!(f.seq, 0x26);
    assert_eq!(f.cmd, cmd::IDLE);
    assert_eq!(f.arg, 0x0000_1b3a); // advancing buffer arg, not a checksum
    assert!(f.body.is_empty());
}

#[test]
fn handshake_frame() {
    let f = round_trip("HANDSHAKE", HANDSHAKE);
    assert_eq!(f.magic, MAGIC_HANDSHAKE); // the special 0x28 magic
    assert_eq!((f.src, f.dst), channel::PRIMARY);
    assert_eq!(f.cmd, cmd::HANDSHAKE);
    assert_eq!(f.body, hex("00100000"));
}

#[test]
fn session_open_tlv() {
    let f = round_trip("SESSION_OPEN_1", SESSION_OPEN_1);
    assert_eq!(f.cmd, cmd::OPEN);
    let tlv = Tlv::parse(&f.body).unwrap();
    assert_eq!(tlv.ty, op::SESSION_OPEN);
    assert_eq!(tlv.value, vec![0x02]); // resource id 2 (frame len strips the LE padding)
}

#[test]
fn bypass_is_op_0x0006_with_handle() {
    let f = round_trip("BYPASS", BYPASS);
    assert_eq!((f.src, f.dst), channel::EDIT);
    assert_eq!(f.cmd, cmd::OPEN);
    let tlv = Tlv::parse(&f.body).unwrap();
    assert_eq!(tlv.ty, op::PARAM_SET); // 0x0006 — the bypass rides the same op
    assert_eq!(tlv.value.len(), 13);
    // context handle that the handshake establishes (not a block address)
    assert_eq!(&tlv.value[0..4], &hex("8366cd03")[..]);
}

#[test]
fn param_set_value_is_big_endian_f32() {
    // Real TLV body from set_tremolo_mix_to_100 (frame 2389): Mix dragged to 100% = 1.0.
    let body = hex("0100060017000000 8366cd0438641e658562041dc31a001c0777ca 3f800000");
    let mut tlv = Tlv::parse(&body).unwrap();
    assert_eq!(tlv.ty, op::PARAM_SET);
    assert_eq!(tlv.value.len(), 0x17); // 23 bytes
    assert_eq!(tlv.trailing_f32(), Some(1.0)); // 3f800000 BE == 1.0

    // We can retarget the value while preserving the handle prefix.
    tlv.set_trailing_f32(0.0);
    assert_eq!(tlv.trailing_f32(), Some(0.0));
    assert_eq!(&tlv.value[tlv.value.len() - 4..], &[0, 0, 0, 0]);

    // And the command TLV re-encodes cleanly.
    let rebuilt = Tlv::command(op::PARAM_SET, tlv.value.clone());
    assert_eq!(Tlv::parse(&rebuilt.to_bytes()).unwrap(), rebuilt);
}

#[test]
fn rejects_truncated_buffers() {
    assert!(Frame::decode(&hex("0800")).is_err());
}

#[test]
fn stream_chunk_uses_u16_length() {
    // A 272-byte stream chunk (256-byte body) carries len = 0x0108 in the first two bytes,
    // matching real chunk headers like `08 01 00 18 ...` (frame 2425 of the preset-open capture).
    let f = Frame::new(channel::EDIT.1, channel::EDIT.0, 0x7b, cmd::OPEN, 0x0b94, vec![0xAB; 256]);
    let wire = f.encode();
    assert_eq!(wire.len(), 272);
    assert_eq!(&wire[0..2], &[0x08, 0x01]); // 264 = 256 body + 8, little-endian u16
    assert_eq!(Frame::decode(&wire).unwrap(), f); // round-trips
}

#[test]
fn generates_primary_handshake_byte_exact() {
    // the validated 5-packet primary handshake. Proves our typed
    // API generates correct wire bytes, not just parses them.
    let expected = [
        "0C0000280110EF03000000020001002100100000",
        "110000180110EF030002000400100000010002000100000002000000",
        "080000180110EF030003000809100000",
        "1A0000180110EF030004000409100000010002000A0000008366CD03E864CCFE6580 0000",
        "080000180110EF03000500081A100000",
    ];
    let frames = fretwire_protocol::session::primary_handshake();
    assert_eq!(frames.len(), expected.len());
    for (i, (f, want)) in frames.iter().zip(expected).enumerate() {
        assert_eq!(f.encode(), hex(want), "handshake packet {} mismatch", i + 1);
    }
}
