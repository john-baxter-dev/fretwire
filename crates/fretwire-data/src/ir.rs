//! The device's user **impulse response** store: what a slot holds, and the WAV files either side.
//!
//! An HX Stomp keeps 128 user IR slots. Each holds a fixed **2048-sample mono impulse**, sent on
//! the wire as 8192 bytes of little-endian `f32` — no header, no sample rate, no channel count.
//! The device runs at 48 kHz, and HX Edit's own `.hxb` backup writes these slots out as 32-bit
//! float 48 kHz mono RIFF WAV, which is where the numbers this module assumes come from.
//!
//! The wire side (which opcodes carry this) lives in `fretwire_protocol::edit`; this module is the
//! part that needs no device: parsing the metadata the device answers with, and converting a blob
//! to and from a file a person can actually listen to.

use crate::rmpv::Value;
use crate::stream::{map_get, value_bytes};

/// Samples in one IR: the device's fixed length.
pub const IR_SAMPLES: usize = 2048;
/// Bytes in one IR blob — [`IR_SAMPLES`] little-endian `f32`.
pub const IR_BLOB_LEN: usize = IR_SAMPLES * 4;
/// The sample rate the device runs at, and the one written into an exported WAV.
pub const IR_SAMPLE_RATE: u32 = 48_000;
/// User IR slots on an HX Stomp. A Floor has the same 128. [solid — `.hxb` carries 128 slots]
pub const IR_SLOTS: usize = 128;

/// One slot's metadata, as the device answers it.
///
/// The device replies to a slot select (op 12) with all of this, and to a commit (op 13) with an
/// array of the same records for the populated slots — so the whole store can be read without
/// moving a single blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrSlot {
    /// Zero-based slot index.
    pub index: i64,
    /// The stored checksum of the slot's blob, when the reply carries one (`113`). The directory
    /// form of the reply omits it.
    pub checksum: Option<u32>,
    /// The slot's name, NUL-padding stripped. Empty for an unused slot.
    pub name: String,
    /// Flags `114/115/123/124/125`, in that order. See [`IrFlags`].
    pub flags: IrFlags,
}

impl IrSlot {
    /// Whether the slot holds an IR.
    ///
    /// Flag `114` is the device's own answer and is what this trusts; the name is a fallback, for
    /// a device that turns out to report occupancy differently. The two can disagree, and the
    /// difference is not academic: a slot holding a **nameless silent IR** reads as empty by name
    /// and as occupied by flag, and it is occupied — assigning it to an IR block gives silence,
    /// not a bypass.
    pub fn is_used(&self) -> bool {
        self.flags.f114 != 0 || !self.name.is_empty()
    }

    /// A name for display, standing in for the nameless.
    pub fn display_name(&self) -> &str {
        match (self.name.as_str(), self.is_used()) {
            ("", true) => "(unnamed)",
            ("", false) => "—",
            (n, _) => n,
        }
    }
}

/// The flags an IR record carries. See [`IrSlot::flags`].
///
/// Two of these were read as constants for as long as every sample came from a populated slot.
/// Reading an **empty** one separates them: `114: 0, 115: 1` where a populated slot says
/// `114: 1, 115: 3`. [solid — live on an HX Stomp 2026-08-22, an untouched slot beside one we had
/// just written]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IrFlags {
    /// Key 114 — **whether the slot holds an IR**: `1` populated, `0` empty. [solid]
    pub f114: i64,
    /// Key 115 — `3` populated, `1` empty. Tracks occupancy too, so what it adds over `114` is
    /// unknown; a slot format or a source kind would both fit. [hypothesis]
    pub f115: i64,
    /// Key 123 — `false` on every slot seen, empty or full.
    pub f123: bool,
    /// Key 124 — `false` on every slot seen, empty or full.
    pub f124: bool,
    /// Key 125 — `0` on every slot seen, empty or full.
    pub f125: i64,
}

/// Strip a fixed-width NUL-padded name field down to its text.
fn trim_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

/// Parse one IR metadata record — the `104` payload of a select reply, or one element of a commit
/// reply's array.
///
/// Returns `None` only if there is no slot index to hang it on; everything else has a default,
/// because a record for an empty slot legitimately omits most of it.
pub fn parse_slot(v: &Value) -> Option<IrSlot> {
    let index = map_get(v, 112).and_then(Value::as_i64)?;
    Some(IrSlot {
        index,
        checksum: map_get(v, 113).and_then(Value::as_u64).map(|n| n as u32),
        name: map_get(v, 109)
            .and_then(value_bytes)
            .map_or_else(String::new, trim_name),
        flags: IrFlags {
            f114: map_get(v, 114).and_then(Value::as_i64).unwrap_or_default(),
            f115: map_get(v, 115).and_then(Value::as_i64).unwrap_or_default(),
            f123: map_get(v, 123).and_then(Value::as_bool).unwrap_or_default(),
            f124: map_get(v, 124).and_then(Value::as_bool).unwrap_or_default(),
            f125: map_get(v, 125).and_then(Value::as_i64).unwrap_or_default(),
        },
    })
}

/// Parse a commit reply's directory: the `104` payload as an array of records.
pub fn parse_directory(v: &Value) -> Vec<IrSlot> {
    match v {
        Value::Array(items) => items.iter().filter_map(parse_slot).collect(),
        _ => Vec::new(),
    }
}

/// The blob's samples, as `f32`.
pub fn samples(blob: &[u8]) -> Vec<f32> {
    blob.as_chunks::<4>()
        .0
        .iter()
        .map(|w| f32::from_le_bytes(*w))
        .collect()
}

/// The peak absolute sample, for a one-glance sanity check that a blob is really an impulse and
/// not a slot full of zeroes.
pub fn peak(blob: &[u8]) -> f32 {
    samples(blob)
        .into_iter()
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Wrap an IR blob as a 32-bit float, 48 kHz, mono RIFF WAV — the format HX Edit's own `.hxb`
/// backup stores these in, so an exported file drops straight into any other IR loader.
pub fn to_wav(blob: &[u8]) -> Vec<u8> {
    let data_len = blob.len() as u32;
    // `fmt ` is 16 bytes for PCM but 18 for IEEE float (a `cbSize` of 0), and players are stricter
    // about that than they look.
    let fmt_len: u32 = 18;
    let riff_len = 4 + (8 + fmt_len) + (8 + data_len);
    let mut out = Vec::with_capacity(riff_len as usize + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&fmt_len.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // WAVE_FORMAT_IEEE_FLOAT
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&IR_SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(IR_SAMPLE_RATE * 4).to_le_bytes()); // byte rate
    out.extend_from_slice(&4u16.to_le_bytes()); // block align
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(blob);
    out
}

/// What went wrong turning a file into something the device will take.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WavError {
    /// Not a RIFF/WAVE container at all.
    #[error("not a RIFF/WAVE file")]
    NotWav,
    /// A chunk header ran past the end of the file.
    #[error("truncated WAV: a chunk header runs past the end of the file")]
    Truncated,
    /// No `fmt ` chunk, or one too short to read.
    #[error("WAV has no usable `fmt ` chunk")]
    NoFmt,
    /// No `data` chunk.
    #[error("WAV has no `data` chunk")]
    NoData,
    /// A format this converter does not read.
    #[error("unsupported WAV format: {0}")]
    Unsupported(String),
    /// The file holds no samples.
    #[error("WAV holds no samples")]
    Empty,
}

/// A WAV's format, as far as this converter cares.
struct WavFmt {
    format: u16,
    channels: u16,
    rate: u32,
    bits: u16,
}

/// Walk the RIFF chunk list, returning the `fmt ` fields and the `data` bytes.
fn read_chunks(bytes: &[u8]) -> Result<(WavFmt, &[u8]), WavError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWav);
    }
    let mut fmt = None;
    let mut data = None;
    let mut off = 12;
    while off + 8 <= bytes.len() {
        let id = &bytes[off..off + 4];
        let len = u32::from_le_bytes([
            bytes[off + 4],
            bytes[off + 5],
            bytes[off + 6],
            bytes[off + 7],
        ]) as usize;
        let body = bytes
            .get(off + 8..off + 8 + len)
            .ok_or(WavError::Truncated)?;
        match id {
            b"fmt " if body.len() >= 16 => {
                // WAVE_FORMAT_EXTENSIBLE (0xfffe) says nothing by itself — the real format tag is
                // the first two bytes of the SubFormat GUID at offset 24. Without unwrapping it, a
                // 32-bit *float* extensible file is indistinguishable from 32-bit *integer* PCM,
                // and every sample comes out scaled by 2^31.
                let mut format = u16::from_le_bytes([body[0], body[1]]);
                if format == 0xfffe && body.len() >= 26 {
                    format = u16::from_le_bytes([body[24], body[25]]);
                }
                fmt = Some(WavFmt {
                    format,
                    channels: u16::from_le_bytes([body[2], body[3]]),
                    rate: u32::from_le_bytes([body[4], body[5], body[6], body[7]]),
                    bits: u16::from_le_bytes([body[14], body[15]]),
                });
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are word-aligned: an odd length is followed by a pad byte.
        off += 8 + len + (len & 1);
    }
    Ok((fmt.ok_or(WavError::NoFmt)?, data.ok_or(WavError::NoData)?))
}

/// Read a WAV into the device's blob format: 2048 mono `f32` samples, little-endian.
///
/// Accepts 16-bit and 24-bit PCM and 32-bit float, mono or multi-channel (channels past the first
/// are dropped, which is what an IR loader does with a stereo file). The sample **rate is not
/// converted** — a 44.1 kHz IR loaded at 48 kHz plays slightly short and bright, so the caller is
/// told the rate and decides. Longer files are truncated to [`IR_SAMPLES`] and shorter ones are
/// zero-padded, exactly as a fixed-length convolver requires.
///
/// Returns the blob and the file's own sample rate.
pub fn from_wav(bytes: &[u8]) -> Result<(Vec<u8>, u32), WavError> {
    let (fmt, data) = read_chunks(bytes)?;
    let channels = fmt.channels.max(1) as usize;
    // 1 = PCM, 3 = IEEE float; extensible has already been resolved to one of those.
    let frames: Vec<f32> = match (fmt.format, fmt.bits) {
        (1, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|w| i16::from_le_bytes(*w) as f32 / 32768.0)
            .collect(),
        (1, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|w| (i32::from_le_bytes([0, w[0], w[1], w[2]]) >> 8) as f32 / 8_388_608.0)
            .collect(),
        (1, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| i32::from_le_bytes(*w) as f32 / 2_147_483_648.0)
            .collect(),
        (3, 32) => samples(data),
        (f, b) => {
            return Err(WavError::Unsupported(format!(
                "format tag {f}, {b}-bit — this reads 16/24-bit PCM and 32-bit float"
            )));
        }
    };
    if frames.is_empty() {
        return Err(WavError::Empty);
    }
    let mut blob = Vec::with_capacity(IR_BLOB_LEN);
    for i in 0..IR_SAMPLES {
        let s = frames.get(i * channels).copied().unwrap_or(0.0);
        blob.extend_from_slice(&s.to_le_bytes());
    }
    Ok((blob, fmt.rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rmpv::Value;

    fn record() -> Value {
        Value::Map(vec![
            (Value::from(112), Value::from(3)),
            (Value::from(113), Value::from(3_231_741_677u32)),
            (
                Value::from(109),
                Value::from("G12-65 212 C Hi-Gn 421+57 Celes\0"),
            ),
            (Value::from(114), Value::from(1)),
            (Value::from(115), Value::from(3)),
            (Value::from(123), Value::from(false)),
            (Value::from(124), Value::from(false)),
            (Value::from(125), Value::from(0)),
        ])
    }

    #[test]
    fn a_slot_record_decodes_to_its_name_and_checksum() {
        let s = parse_slot(&record()).unwrap();
        assert_eq!(s.index, 3);
        assert_eq!(s.checksum, Some(0xc0a0_76ed));
        assert_eq!(s.name, "G12-65 212 C Hi-Gn 421+57 Celes");
        assert!(s.is_used());
        assert_eq!(s.flags.f114, 1);
        assert_eq!(s.flags.f115, 3);
    }

    #[test]
    fn an_empty_slot_is_a_record_the_occupancy_flag_calls_empty() {
        let v = Value::Map(vec![
            (Value::from(112), Value::from(9)),
            (Value::from(114), Value::from(0)),
            (Value::from(115), Value::from(1)),
        ]);
        let s = parse_slot(&v).unwrap();
        assert_eq!(s.index, 9);
        assert!(!s.is_used());
        assert_eq!(s.display_name(), "—");
    }

    #[test]
    fn a_nameless_slot_the_flag_calls_full_is_full() {
        // What a zero-filled "clear" leaves behind: no name, but the device still counts it as
        // holding an IR — a silent one. Reading occupancy off the name alone would call it empty
        // and quietly overwrite it, or offer it as free space it is not.
        let v = Value::Map(vec![
            (Value::from(112), Value::from(1)),
            (Value::from(113), Value::from(0)),
            (Value::from(114), Value::from(1)),
            (Value::from(115), Value::from(3)),
        ]);
        let s = parse_slot(&v).unwrap();
        assert!(s.is_used());
        assert_eq!(s.display_name(), "(unnamed)");
    }

    #[test]
    fn a_record_without_a_slot_index_is_not_a_record() {
        assert!(parse_slot(&Value::Map(vec![(Value::from(109), Value::from("x"))])).is_none());
        assert!(parse_slot(&Value::Nil).is_none());
    }

    #[test]
    fn a_directory_keeps_only_the_records() {
        let dir = Value::Array(vec![record(), Value::Nil, record()]);
        assert_eq!(parse_directory(&dir).len(), 2);
        assert!(parse_directory(&Value::Nil).is_empty());
    }

    #[test]
    fn a_blob_round_trips_through_a_wav() {
        let mut blob = vec![0u8; IR_BLOB_LEN];
        blob[..4].copy_from_slice(&1.0f32.to_le_bytes());
        blob[4..8].copy_from_slice(&(-0.5f32).to_le_bytes());
        let wav = to_wav(&blob);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        // The declared RIFF length must match what actually follows it, or players reject the file.
        assert_eq!(
            u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]) as usize,
            wav.len() - 8
        );
        let (back, rate) = from_wav(&wav).unwrap();
        assert_eq!(back, blob);
        assert_eq!(rate, IR_SAMPLE_RATE);
        assert_eq!(peak(&back), 1.0);
    }

    #[test]
    fn a_short_wav_is_padded_and_a_long_one_is_cut() {
        let short = to_wav(&vec![0u8; 400]);
        let (blob, _) = from_wav(&short).unwrap();
        assert_eq!(blob.len(), IR_BLOB_LEN);
        let long = to_wav(&vec![0u8; IR_BLOB_LEN * 2]);
        let (blob, _) = from_wav(&long).unwrap();
        assert_eq!(blob.len(), IR_BLOB_LEN);
    }

    #[test]
    fn a_stereo_wav_keeps_the_left_channel() {
        // Two interleaved channels: left ramps, right is silent.
        let mut data = Vec::new();
        for i in 0..64 {
            data.extend_from_slice(&(i as f32).to_le_bytes());
            data.extend_from_slice(&0.0f32.to_le_bytes());
        }
        let mut wav = to_wav(&data);
        wav[22] = 2; // channels := 2
        let (blob, _) = from_wav(&wav).unwrap();
        let s = samples(&blob);
        assert_eq!(&s[..4], &[0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn sixteen_bit_pcm_is_scaled_to_unity() {
        let mut wav = to_wav(&[]);
        wav[20] = 1; // format := PCM
        wav[34] = 16; // bits := 16
        wav.extend_from_slice(&i16::MAX.to_le_bytes());
        wav.extend_from_slice(&(-16384i16).to_le_bytes());
        let data_len = 4u32;
        let n = wav.len();
        wav[n - 4 - data_len as usize..n - data_len as usize]
            .copy_from_slice(&data_len.to_le_bytes());
        let riff = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&riff.to_le_bytes());
        let (blob, _) = from_wav(&wav).unwrap();
        let s = samples(&blob);
        assert!((s[0] - 1.0).abs() < 1e-4);
        assert!((s[1] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_file_that_is_not_a_wav_says_so() {
        assert_eq!(from_wav(b"not a wav at all").unwrap_err(), WavError::NotWav);
        assert_eq!(from_wav(&[]).unwrap_err(), WavError::NotWav);
    }

    #[test]
    fn an_unsupported_depth_names_itself() {
        let mut wav = to_wav(&[0u8; 8]);
        wav[34] = 8; // bits := 8
        assert!(matches!(
            from_wav(&wav).unwrap_err(),
            WavError::Unsupported(_)
        ));
    }
}
