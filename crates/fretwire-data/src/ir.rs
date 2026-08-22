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
    /// The MD5 of the stored samples, lowercase hex, when the record carries one (key `104`).
    /// Only a directory listing does; a slot select does not.
    pub md5: Option<String>,
    /// Key `114`, the length **multiplier**. See [`IrSlot::stored_samples`].
    pub length_mul: i64,
    /// Key `115`, the length **exponent**. See [`IrSlot::stored_samples`].
    pub length_exp: i64,
    /// Keys `123/124/125`, echoed back verbatim. See [`IrFlags`].
    pub flags: IrFlags,
}

impl IrSlot {
    /// How many samples the device stores here: **`114 x 256 x 2^115`**.
    ///
    /// An empty slot reports `0, 1` and so stores nothing, which is why these two once looked like
    /// an occupancy flag — the zero is a length, not a boolean.
    pub fn stored_samples(&self) -> i64 {
        // A garbage exponent off the wire must not shift out of range.
        let exp = self.length_exp.clamp(0, 31) as u32;
        self.length_mul
            .saturating_mul(256)
            .saturating_mul(1i64 << exp)
    }

    /// Whether the slot holds an IR.
    ///
    /// A stored length of zero is the device's own answer, and the name is a fallback. The two can
    /// disagree, and the difference is not academic: a slot holding a **nameless silent IR** reads
    /// as empty by name and as occupied by length, and it is occupied — assigning it to an IR
    /// block gives silence, not a bypass.
    pub fn is_used(&self) -> bool {
        self.stored_samples() > 0 || !self.name.is_empty()
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

/// Keys `123/124/125`, which an IR record echoes back verbatim.
///
/// Not IR-specific — preset list entries carry the same trio, `false, false, 0` everywhere seen.
/// Their meaning is untested. (Keys `114`/`115`, which used to live here as "format flags", are a
/// declared length and moved to [`IrSlot::stored_samples`].)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IrFlags {
    /// Key 123.
    pub f123: bool,
    /// Key 124.
    pub f124: bool,
    /// Key 125.
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
        md5: map_get(v, 104)
            .and_then(value_bytes)
            .map(|b| trim_name(b).to_ascii_lowercase())
            .filter(|s| s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())),
        length_mul: map_get(v, 114).and_then(Value::as_i64).unwrap_or_default(),
        length_exp: map_get(v, 115).and_then(Value::as_i64).unwrap_or_default(),
        flags: IrFlags {
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

/// MD5, for the one field the device reports it in.
///
/// Each entry in the IR directory carries key `104`: the MD5 of the slot's stored sample bytes,
/// *after* the device's zero-padding, as lowercase hex. That makes end-to-end verification of an
/// upload free and far stronger than the `113` word sum, which any reordering of the samples
/// collides with. Hand-rolled rather than pulling in a dependency for one 64-round function.
///
/// This is a checksum for comparing against a value the device computed, not a security primitive.
pub fn md5_hex(data: &[u8]) -> String {
    /// Per-round left-rotation amounts.
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // K[i] = floor(2^32 * abs(sin(i + 1))), the constants from RFC 1321.
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];

    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let [mut a0, mut b0, mut c0, mut d0] = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];

    for block in msg.as_chunks::<64>().0 {
        let m: [u32; 16] = std::array::from_fn(|i| {
            u32::from_le_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ])
        });
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let tmp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                f.wrapping_add(a)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = tmp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = String::with_capacity(32);
    for word in [a0, b0, c0, d0] {
        for byte in word.to_le_bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
    }
    out
}

/// The MD5 the device will report for `blob` once stored in a slot of `stored_samples` samples.
///
/// The device zero-pads what it is given up to the declared length and hashes *that*, so a short
/// upload's hash is not the hash of the file.
pub fn stored_md5(blob: &[u8], stored_samples: usize) -> String {
    let want = stored_samples * 4;
    if blob.len() >= want {
        md5_hex(&blob[..want])
    } else {
        let mut padded = blob.to_vec();
        padded.resize(want, 0);
        md5_hex(&padded)
    }
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
        assert_eq!(s.stored_samples(), 2048);
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
        assert_eq!(s.stored_samples(), 0);
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
        assert_eq!(s.stored_samples(), 2048);
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

    #[test]
    fn md5_matches_the_rfc_1321_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"a"), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"message digest"),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            md5_hex(b"abcdefghijklmnopqrstuvwxyz"),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
        assert_eq!(
            md5_hex(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"),
            "d174ab98d277d9f5a5611c2c9f419d9f"
        );
        // Spans a block boundary — 80 bytes, so two blocks with the length in the second.
        assert_eq!(
            md5_hex(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    #[test]
    fn the_stored_hash_covers_the_devices_zero_padding() {
        // Half an IR stored in a full-length slot hashes as itself plus 4 KB of zeros — not as
        // the file, which is the trap this exists to avoid.
        let half = vec![7u8; 4096];
        let mut padded = half.clone();
        padded.resize(IR_BLOB_LEN, 0);
        assert_eq!(stored_md5(&half, IR_SAMPLES), md5_hex(&padded));
        // Given exactly the stored length, it is the plain hash.
        assert_eq!(stored_md5(&padded, IR_SAMPLES), md5_hex(&padded));
    }

    #[test]
    fn a_directory_entry_carries_its_hash() {
        let v = Value::Map(vec![
            (Value::from(112), Value::from(2)),
            (Value::from(109), Value::from("x\0")),
            (Value::from(114), Value::from(1)),
            (Value::from(115), Value::from(3)),
            (
                Value::from(104),
                Value::from("D41D8CD98F00B204E9800998ECF8427E\0"),
            ),
        ]);
        // Lowercased, NUL stripped — the device sends it padded like a name.
        assert_eq!(
            parse_slot(&v).unwrap().md5.as_deref(),
            Some("d41d8cd98f00b204e9800998ecf8427e")
        );
    }

    #[test]
    fn a_hash_field_that_is_not_a_hash_is_dropped() {
        let v = Value::Map(vec![
            (Value::from(112), Value::from(2)),
            (Value::from(104), Value::from("not a hash")),
        ]);
        assert_eq!(parse_slot(&v).unwrap().md5, None);
    }
}
