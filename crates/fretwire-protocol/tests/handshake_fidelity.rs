//! `device_handshake()` must reproduce, byte-exact, the frames HX Edit sends to bring up a session
//! on this unit. Reference bytes captured from `startup.pcapng` (full frames, EP 0x01 OUT).

use fretwire_protocol::session::device_handshake;

fn hx(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn device_handshake_is_byte_exact() {
    // Channel-grouped order: ef03 (F1-F4), ed03 (F1-F3), f003 (F1-F3).
    let expected = [
        // primary ef03
        "0c0000280110ef03000000020001002100100000",
        "110000180110ef030002000400100000010005000100000005000000",
        "080000180110ef030003000820100000",
        "080000180110ef030004000220100000",
        // edit ed03
        "0c0000288010ed03000000020001002100100000",
        "110000188010ed030002000400100000010006000100000006000000",
        "080000188010ed030003000809100000",
        // status f003
        "0c0000280210f003000000020001002100100000",
        "110000180210f0030002000400100000010004000100000004000000",
        "080000180210f0030003000809100000",
    ];
    let frames = device_handshake();
    assert_eq!(frames.len(), expected.len(), "frame count");
    for (i, (f, want)) in frames.iter().zip(expected).enumerate() {
        assert_eq!(f.encode(), hx(want), "handshake frame {i} differs from capture");
    }
}
