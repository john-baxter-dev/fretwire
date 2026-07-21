//! The TLV body carried by data frames.
//!
//! Layout: `marker:u16 LE`, `type:u16 LE`, `len:u32 LE`, `value[len]`.
//! The `type` is the sub-command (see `crate::op`); `value` is its payload. For a parameter
//! set (`type == op::PARAM_SET`) the value ends with the new setting as a **big-endian f32**.

use crate::{u16le, u32le, Error, Result};

/// Marker on a host command body (`01 00`).
pub const TLV_MARKER_CMD: u16 = 0x0001;
/// Marker on a device reply body (`00 00`).
pub const TLV_MARKER_REPLY: u16 = 0x0000;

const TLV_HEADER_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tlv {
    pub marker: u16,
    pub ty: u16,
    pub value: Vec<u8>,
}

impl Tlv {
    /// A host command TLV (`marker = 01 00`).
    pub fn command(ty: u16, value: Vec<u8>) -> Self {
        Tlv { marker: TLV_MARKER_CMD, ty, value }
    }

    /// Parse a TLV from a frame's significant `body` bytes.
    pub fn parse(body: &[u8]) -> Result<Tlv> {
        if body.len() < TLV_HEADER_LEN {
            return Err(Error::NotTlv(body.len()));
        }
        let len = u32le(&body[4..8]) as usize;
        let end = TLV_HEADER_LEN + len;
        if end > body.len() {
            return Err(Error::BadLength { declared: end, avail: body.len() });
        }
        Ok(Tlv {
            marker: u16le(&body[0..2]),
            ty: u16le(&body[2..4]),
            value: body[TLV_HEADER_LEN..end].to_vec(),
        })
    }

    /// Encode to bytes suitable for use as a [`crate::Frame`] body.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(TLV_HEADER_LEN + self.value.len());
        out.extend_from_slice(&self.marker.to_le_bytes());
        out.extend_from_slice(&self.ty.to_le_bytes());
        out.extend_from_slice(&(self.value.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.value);
        out
    }

    /// The trailing big-endian `f32`, if the value is long enough — the new parameter value
    /// on a `PARAM_SET`. Returns `None` for value-less ops (e.g. bypass).
    pub fn trailing_f32(&self) -> Option<f32> {
        let n = self.value.len();
        if n < 4 {
            return None;
        }
        Some(f32::from_be_bytes([
            self.value[n - 4],
            self.value[n - 3],
            self.value[n - 2],
            self.value[n - 1],
        ]))
    }

    /// Replace (or append) the trailing big-endian `f32` value — used to build a `PARAM_SET`
    /// at a known value while preserving the leading handle bytes.
    pub fn set_trailing_f32(&mut self, v: f32) {
        let n = self.value.len();
        let be = v.to_be_bytes();
        if n >= 4 {
            self.value[n - 4..].copy_from_slice(&be);
        } else {
            self.value.extend_from_slice(&be);
        }
    }
}
