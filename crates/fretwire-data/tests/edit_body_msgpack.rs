//! PROOF: the edit-channel command body (the inner `data` after the TLV header) is **MessagePack**
//! — the same encoding as the preset stream. The "opaque handle" `83 66 cd …` is really a msgpack
//! map. Decoded straight from captured bytes (docs/protocol.md, tools/dump-control.ps1).

use fretwire_data::stream::map_get;
use rmpv::Value;

// Bypass toggle, tremolo (capture toggle_tremolo_on_off.pcapng, frame 1479). TLV data (ilen 13).
const BYPASS_ON: &[u8] = &[
    0x83, 0x66, 0xcd, 0x03, 0xf2, 0x64, 0x29, 0x65, 0x82, 0x62, 0x04, 0x3b, 0xc3,
];
// Same edit, next frame (2095): the bypass value flips true->false (c3 -> c2).
const BYPASS_OFF: &[u8] = &[
    0x83, 0x66, 0xcd, 0x03, 0xf3, 0x64, 0x29, 0x65, 0x82, 0x62, 0x04, 0x3b, 0xc2,
];
// Set tremolo Mix = 0.0 (set_tremolo_mix_to_100.pcapng, frame 1731). TLV data (ilen 23).
const SET_MIX_0: &[u8] = &[
    0x83, 0x66, 0xcd, 0x04, 0x27, 0x64, 0x1e, 0x65, 0x85, 0x62, 0x04, 0x1d, 0xc3, 0x1a, 0x00, 0x1c,
    0x07, 0x77, 0xca, 0x00, 0x00, 0x00, 0x00,
];
// Set tremolo Mix = 0.2 (frame 1951): value f32 BE = 3e4ccccd.
const SET_MIX_02: &[u8] = &[
    0x83, 0x66, 0xcd, 0x04, 0x2c, 0x64, 0x1e, 0x65, 0x85, 0x62, 0x04, 0x1d, 0xc3, 0x1a, 0x00, 0x1c,
    0x07, 0x77, 0xca, 0x3e, 0x4c, 0xcc, 0xcd,
];

fn decode(data: &[u8]) -> Value {
    let mut cur = data;
    let v = rmpv::decode::read_value(&mut cur).expect("edit body must be valid MessagePack");
    assert!(cur.is_empty(), "edit body had {} trailing bytes", cur.len());
    v
}

#[test]
fn edit_bodies_are_messagepack_maps() {
    for d in [BYPASS_ON, BYPASS_OFF, SET_MIX_0, SET_MIX_02] {
        let v = decode(d);
        assert!(matches!(v, Value::Map(_)), "root should be a map");
        // Outer envelope keys: 102 (txn), 100 (op/param tag), 101 (target).
        assert!(map_get(&v, 102).is_some());
        assert!(map_get(&v, 100).is_some());
        assert!(map_get(&v, 101).is_some());
    }
}

#[test]
fn slot_is_inner_key_98() {
    // The block address (slot) lives at inner key 98 in every edit — tremolo = slot 4.
    for d in [BYPASS_ON, SET_MIX_0] {
        let v = decode(d);
        let target = map_get(&v, 101).unwrap();
        assert_eq!(map_get(target, 98).and_then(Value::as_i64), Some(4));
    }
}

#[test]
fn bypass_is_set_state_bool_at_key_59() {
    // Resolves the old open question: bypass is NOT a blind toggle — it carries an explicit bool
    // at inner key 59 (on=true, off=false across the two frames of one user toggle).
    let on = decode(BYPASS_ON);
    let off = decode(BYPASS_OFF);
    let bv = |v: &Value| map_get(map_get(v, 101).unwrap(), 59).and_then(Value::as_bool);
    assert_eq!(bv(&on), Some(true));
    assert_eq!(bv(&off), Some(false));
}

#[test]
fn param_value_is_f32_at_inner_key_119() {
    // The set-value payload is a big-endian float32 at inner key 119 (msgpack 0xca).
    let v0 = decode(SET_MIX_0);
    let v02 = decode(SET_MIX_02);
    let fv = |v: &Value| match map_get(map_get(v, 101).unwrap(), 119) {
        Some(Value::F32(f)) => Some(*f),
        _ => None,
    };
    assert_eq!(fv(&v0), Some(0.0));
    assert_eq!(fv(&v02), Some(0.2));
}

#[test]
fn key_102_is_a_u16_counter_and_op_lives_at_key_100() {
    // Correction from many single-knob captures: key 102 is a *whole* u16 running counter (the same
    // param edited later shows 0x04xx/0x05xx — the high byte is NOT an op class). The operation is
    // identified by key 100: 41 = bypass, 30 = set-value.
    let on = decode(BYPASS_ON); // counter 0x03f2
    let off = decode(BYPASS_OFF); // 0x03f3
    let mix = decode(SET_MIX_0);
    let k = |v: &Value, key: i64| map_get(v, key).and_then(Value::as_i64).unwrap();
    assert_eq!(k(&off, 102), k(&on, 102) + 1); // counter increments by 1
    assert_eq!(k(&on, 100), 41); // bypass op
    assert_eq!(k(&mix, 100), 30); // set-value op
}

#[test]
fn param_is_selected_by_index_at_key_28() {
    // The set-value target selects the parameter by its index (key 28) in the model's device order.
    // Dynamic Ambience Mix is index 5 (order: RoomSize,PreDelay,Damping,Diffusion,EarlyLateBlend,
    // Mix,LowCut,HighCut,Level); LowCut is index 6. (captures dynamic_ambience_*_modify)
    let ambience_mix: &[u8] = &[
        0x83, 0x66, 0xcd, 0x04, 0xa1, 0x64, 0x1e, 0x65, 0x85, 0x62, 0x07, 0x1d, 0xc3, 0x1a, 0x00,
        0x1c, 0x05, 0x77, 0xca, 0x3f, 0x1e, 0xb8, 0x52,
    ];
    let ambience_lowcut: &[u8] = &[
        0x83, 0x66, 0xcd, 0x04, 0x49, 0x64, 0x1e, 0x65, 0x85, 0x62, 0x07, 0x1d, 0xc3, 0x1a, 0x00,
        0x1c, 0x06, 0x77, 0xca, 0x43, 0x45, 0xff, 0xff,
    ];
    let idx = |d: &[u8]| {
        let v = decode(d);
        map_get(map_get(&v, 101).unwrap(), 28).and_then(Value::as_i64)
    };
    assert_eq!(idx(ambience_mix), Some(5)); // Mix
    assert_eq!(idx(ambience_lowcut), Some(6)); // LowCut
}
