//! The 16-byte frame header + body, with exact-byte encode/decode.
//!
//! Layout (integers little-endian unless noted):
//! ```text
//! off size field   notes
//! 0   2    len     u16 LE: 8 + significant-body-len (excludes trailing zero padding). For
//!                  small frames the high byte is 0 (looks like a u8); stream chunks use it
//!                  (e.g. a 272-byte chunk has len = 0x0108 = 264).
//! 2   1    0x00
//! 3   1    magic   0x18 normal, 0x28 for the first HANDSHAKE packet
//! 4   2    src     host channel id
//! 6   2    dst     device channel id
//! 8   1    0x00
//! 9   1    seq     per-channel sequence counter
//! 10  1    0x00
//! 11  1    cmd     see `crate::cmd`
//! 12  4    arg     command-specific u32 (stream offset / buffer address; NOT a checksum)
//! 16  ..   body    significant bytes only; frame is zero-padded to a 4-byte boundary
//! ```

use crate::{Error, Result, u16le, u32le};

/// Standard frame magic (header offset 3).
pub const MAGIC: u8 = 0x18;
/// Magic used only by the first HANDSHAKE packet of a session.
pub const MAGIC_HANDSHAKE: u8 = 0x28;

const HEADER_LEN: usize = 16;
/// The `len` byte counts from offset 8, so it includes these 8 header bytes plus the body.
const LEN_HEADER_TAIL: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub magic: u8,
    pub src: u16,
    pub dst: u16,
    pub seq: u8,
    pub cmd: u8,
    pub arg: u32,
    /// Significant body bytes (no trailing padding). May be empty (idle/chunk frames),
    /// a raw blob (handshake), or a [`crate::Tlv`] (data frames) — parse with `Tlv::parse`.
    pub body: Vec<u8>,
}

impl Frame {
    /// Build a standard (`0x18` magic) frame.
    pub fn new(src: u16, dst: u16, seq: u8, cmd: u8, arg: u32, body: Vec<u8>) -> Self {
        Frame {
            magic: MAGIC,
            src,
            dst,
            seq,
            cmd,
            arg,
            body,
        }
    }

    /// Decode one frame from a bulk transfer buffer. Trailing padding is ignored.
    pub fn decode(buf: &[u8]) -> Result<Frame> {
        if buf.len() < HEADER_LEN {
            return Err(Error::Short {
                need: HEADER_LEN,
                got: buf.len(),
            });
        }
        let len = u16::from_le_bytes([buf[0], buf[1]]) as usize;
        // significant body length is `len` minus the 8 header bytes it also covers.
        let body_len = len.checked_sub(LEN_HEADER_TAIL).ok_or(Error::BadLength {
            declared: len,
            avail: buf.len(),
        })?;
        let end = HEADER_LEN + body_len;
        if end > buf.len() {
            return Err(Error::BadLength {
                declared: end,
                avail: buf.len(),
            });
        }
        Ok(Frame {
            magic: buf[3],
            src: u16le(&buf[4..6]),
            dst: u16le(&buf[6..8]),
            seq: buf[9],
            cmd: buf[11],
            arg: u32le(&buf[12..16]),
            body: buf[HEADER_LEN..end].to_vec(),
        })
    }

    /// Decode one frame and report how many wire bytes it consumed (header + body, padded up to
    /// the 4-byte boundary — matching [`Frame::encode`]). Use to walk a bulk-IN buffer that
    /// concatenates several frames.
    pub fn decode_one(buf: &[u8]) -> Result<(Frame, usize)> {
        let frame = Frame::decode(buf)?;
        let raw = HEADER_LEN + frame.body.len();
        let consumed = (raw + 3) & !3; // pad to a 4-byte boundary, as encode() does
        Ok((frame, consumed))
    }

    /// Decode every frame concatenated in a single bulk-IN transfer. A short tail (`< HEADER_LEN`
    /// bytes, i.e. trailing padding) ends the walk.
    pub fn decode_all(buf: &[u8]) -> Result<Vec<Frame>> {
        let mut out = Vec::new();
        let mut rest = buf;
        while rest.len() >= HEADER_LEN {
            let (frame, consumed) = Frame::decode_one(rest)?;
            out.push(frame);
            rest = &rest[consumed.max(HEADER_LEN)..];
        }
        Ok(out)
    }

    /// Encode to wire bytes, zero-padded to a 4-byte boundary (exactly as HX Edit emits).
    pub fn encode(&self) -> Vec<u8> {
        let len = LEN_HEADER_TAIL + self.body.len();
        debug_assert!(
            len <= u16::MAX as usize,
            "frame body too large for the len field"
        );

        let mut out = Vec::with_capacity(HEADER_LEN + self.body.len() + 3);
        out.extend_from_slice(&(len as u16).to_le_bytes());
        out.push(0);
        out.push(self.magic);
        out.extend_from_slice(&self.src.to_le_bytes());
        out.extend_from_slice(&self.dst.to_le_bytes());
        out.push(0);
        out.push(self.seq);
        out.push(0);
        out.push(self.cmd);
        out.extend_from_slice(&self.arg.to_le_bytes());
        out.extend_from_slice(&self.body);
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_one_consumed_matches_encode_len() {
        // A body whose length is not a multiple of 4 forces padding into `consumed`.
        let f = Frame::new(0x1080, 0x03ed, 7, 0x04, 0x23d9, vec![1, 2, 3, 4, 5]);
        let raw = f.encode();
        let (decoded, consumed) = Frame::decode_one(&raw).unwrap();
        assert_eq!(decoded, f);
        assert_eq!(consumed, raw.len());
        assert_eq!(consumed % 4, 0);
    }

    #[test]
    fn decode_all_splits_concatenated_frames() {
        // Mirrors the handshake trace: several frames in one bulk-IN read.
        let frames = [
            Frame::new(0x1080, 0x03ed, 4, 0x04, 0x23d9, vec![0xaa, 0xbb]),
            Frame::new(0x1080, 0x03ed, 5, 0x0c, 0x241a, Vec::new()),
            Frame::new(0x1080, 0x03ed, 6, 0x08, 0x251a, vec![0x11, 0x22, 0x33]),
        ];
        let mut buf = Vec::new();
        for f in &frames {
            buf.extend_from_slice(&f.encode());
        }
        let decoded = Frame::decode_all(&buf).unwrap();
        assert_eq!(decoded, frames);
    }
}
