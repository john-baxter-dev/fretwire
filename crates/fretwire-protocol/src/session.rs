//! The session-init handshake, built from our typed [`Frame`]/[`Tlv`] API.
//!
//! These reproduce the **validated** 5-packet sequence on the primary channel
//! Our own `startup.pcapng` HANDSHAKE packet is byte-identical;
//! the `SESSION_OPEN` resource ids may be session-dependent (our capture showed different ids
//! on other channels), so we follow the known-good reference constants for the primary channel.
//!
//! Usage by the transport layer: send each frame's `encode()` on EP_OUT, read one reply on
//! EP_IN and discard it, in order.

use crate::channel::PRIMARY;
use crate::{cmd, op, Frame, Tlv, MAGIC_HANDSHAKE};

/// The **observed** HX Edit session bring-up for this device (HX Stomp / `P33`), reconstructed
/// byte-exact from `startup.pcapng`. Brings up all three channels — primary (`ef03`), edit
/// (`ed03`), status (`f003`) — each with: a HANDSHAKE open (`0x28` magic, cmd 0x02), an identity
/// query (cmd 0x04, `Tlv(op, [op])`), and a chunk read (cmd 0x08). The identity reply carries the
/// model string (`"P33Main"`/`"P33"`).
///
/// This differs from [`primary_handshake`] (the HX Stomp **XL** sequence, which uses a
/// different op at packet 2); prefer this for first contact since it's what HX Edit sends to *this*
/// unit. Channels are grouped here (capture interleaved them); a strict request/response device
/// should accept the grouped order. Verified byte-exact in tests.
pub fn device_handshake() -> Vec<Frame> {
    // (channel, identity-query op, chunk-read arg). Per-channel chunk arg differs in the capture.
    let chans = [
        (crate::channel::PRIMARY, 0x0005u16, 0x0000_1020u32, true), // ef03 + extra cmd-0x02
        (crate::channel::EDIT, 0x0006, 0x0000_1009, false),         // ed03
        (crate::channel::STATUS, 0x0004, 0x0000_1009, false),       // f003
    ];
    let mut frames = Vec::new();
    for (chan, id_op, chunk_arg, extra_open) in chans {
        let (src, dst) = chan;
        // F1 — HANDSHAKE open (0x28 magic, cmd 0x02, arg 0x21000100, body 00 10 00 00).
        frames.push(Frame {
            magic: MAGIC_HANDSHAKE,
            src,
            dst,
            seq: 0x00,
            cmd: cmd::HANDSHAKE,
            arg: 0x2100_0100,
            body: vec![0x00, 0x10, 0x00, 0x00],
        });
        // F2 — identity query (cmd 0x04, arg 0x1000, Tlv(id_op, [id_op])). Note: seq jumps to 2.
        frames.push(Frame::new(src, dst, 0x02, cmd::OPEN, 0x0000_1000,
            Tlv::command(id_op, vec![id_op as u8]).to_bytes()));
        // F3 — chunk read (cmd 0x08, no body).
        frames.push(Frame::new(src, dst, 0x03, cmd::CHUNK, chunk_arg, Vec::new()));
        // The primary channel also sent a follow-up cmd 0x02 (seq 4, same arg as its chunk read).
        if extra_open {
            frames.push(Frame::new(src, dst, 0x04, cmd::HANDSHAKE, chunk_arg, Vec::new()));
        }
    }
    frames
}

/// The five frames that initialise a session on the primary channel, in send order.
pub fn primary_handshake() -> Vec<Frame> {
    let (src, dst) = PRIMARY;

    // Packet 1 — HANDSHAKE (special 0x28 magic, cmd 0x02, raw body).
    let p1 = Frame {
        magic: MAGIC_HANDSHAKE,
        src,
        dst,
        seq: 0x00,
        cmd: cmd::HANDSHAKE,
        arg: 0x2100_0100,
        body: vec![0x00, 0x10, 0x00, 0x00],
    };

    // Packet 2 — SESSION_OPEN_1: open first resource (TLV type 0x0002, value = resource id 2).
    let p2 = Frame::new(src, dst, 0x02, cmd::OPEN, 0x0000_1000,
        Tlv::command(op::SESSION_OPEN, vec![0x02]).to_bytes());

    // Packet 3 — SESSION_CHUNK_1: chunk request, no body.
    let p3 = Frame::new(src, dst, 0x03, cmd::CHUNK, 0x0000_1009, Vec::new());

    // Packet 4 — SESSION_OPEN_2: open second resource; value carries the context handle.
    let p4 = Frame::new(src, dst, 0x04, cmd::OPEN, 0x0000_1009,
        Tlv::command(op::SESSION_OPEN,
            vec![0x83, 0x66, 0xCD, 0x03, 0xE8, 0x64, 0xCC, 0xFE, 0x65, 0x80]).to_bytes());

    // Packet 5 — SESSION_CHUNK_2: chunk request, no body.
    let p5 = Frame::new(src, dst, 0x05, cmd::CHUNK, 0x0000_101A, Vec::new());

    vec![p1, p2, p3, p4, p5]
}
