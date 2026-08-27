//! Golden tests against real bytes captured from a **POD Go** (issue #15, 2026-08-26).
//!
//! The contributor drove POD Go Edit through one parameter change, one bypass toggle and one block
//! model swap, each on a named slot. These assert that the builders in `fretwire_protocol::edit` —
//! written entirely from HX Stomp captures — reproduce the POD Go's bytes exactly. They do: the
//! write path is the same protocol, not merely a similar one.
//!
//! The bypass body is the sharpest evidence. Against the HX Stomp's captured bypass in `golden.rs`
//! (`8366cd03f16429658262073bc2`) the POD Go's differs in exactly one byte — the slot number.

use fretwire_protocol::edit;

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Toggling bypass on slot 9 (Reverb, Dynamic Room).
/// `{102: 1009, 100: 41, 101: {98: 9, 59: false}}`
#[test]
fn bypass_matches_the_pod_go_bytes() {
    assert_eq!(
        edit::bypass(9, false, 1009),
        hex("8366cd03f16429658262093bc2")
    );
}

/// Changing slot 6 (Amp) parameter 0 — Lead Gain — over a knob drag: 0.75, then 0.3, then the
/// 0.3 again as the drag settles. All three are ordinary main-model float set-values.
/// `{102: txn, 100: 30, 101: {98: 6, 29: true, 26: 0, 28: 0, 119: v}}`
#[test]
fn set_value_matches_the_pod_go_bytes() {
    for (txn, value, wire) in [
        (
            1006u16,
            0.75f32,
            "8366cd03ee641e658562061dc31a001c0077ca3f400000",
        ),
        (1007, 0.3, "8366cd03ef641e658562061dc31a001c0077ca3e99999a"),
        (1008, 0.3, "8366cd03f0641e658562061dc31a001c0077ca3e99999a"),
    ] {
        assert_eq!(edit::set_value(6, 0, value, txn), hex(wire), "txn {txn}");
    }
}

/// Swapping slot 8's model from Elephant Man to Adriatic Delay — model index 75 in `PodGo.sym`,
/// no paired cab. `{102: 1011, 100: 40, 101: {98: 8, 100: {23: false, 25: 75, 26: -1}}}`
#[test]
fn swap_model_matches_the_pod_go_bytes() {
    assert_eq!(
        edit::swap_model(8, 75, -1, 1011),
        hex("8366cd03f3642865826208648317c2194b1aff")
    );
}

/// After the swap POD Go Edit re-reads the block's footswitch and then the whole preset — the same
/// refresh sequence HX Edit performs, built by the same three builders.
#[test]
fn the_post_swap_refresh_matches_the_pod_go_bytes() {
    assert_eq!(edit::read_switch(5, 1012), hex("8366cd03f4642165816605"));
    assert_eq!(edit::read_info(1013), hex("8366cd03f5641765c0"));
    assert_eq!(edit::stream_start(1014), hex("8366cd03f6641665c0"));
}
