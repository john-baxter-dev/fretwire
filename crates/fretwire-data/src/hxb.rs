//! The `.hxb` **device backup** container written by HX Edit.
//!
//! Fully decoded from a Helix Floor backup (see `docs/helix-floor.md`). The header is fixed-layout
//! little-endian; the payload is **concatenated raw zlib streams, back to back** — no index, no
//! length prefixes, so you inflate one and start the next where it ended:
//!
//! ```text
//! 0x00  "AF6L"     magic
//! 0x04  u32        version (1)
//! 0x18  u32        device id  (0x210001 = Helix Floor, matching a preset's own `device`)
//! 0x1c  u32        device version
//! 0x28  u32        unix timestamp
//! 0x30  char[64]   user comment, NUL-padded
//! 0x70  ...        payload
//! ```
//!
//! In the Floor backup the 138 streams are: `#0` globals JSON, `#1..=#128` the 128 IR slots as
//! RIFF WAV, `#129` an `L6UMDArchive` model-usage table, and `#130..=#137` the **eight setlists**
//! (`schema: "L6Setlist"`), each holding exactly 128 preset slots.
//!
//! **This reads a backup; it does not restore one.** A preset inside a `.hxb` is a `tone` **JSON**
//! object, not the MessagePack blob the wire protocol exchanges, so writing one back to the device
//! would need a JSON→blob conversion that does not exist yet. See [`HxbPreset::tone`].
//!
//! Setlist order here is the authority for the `bank` index used by `goto_preset`/`save_preset`:
//! `FACTORY 1, FACTORY 2, USER 1..USER 5, TEMPLATES`. Cross-checked against live wire traffic — a
//! read-info reply reported `PresetInfo { bank: 2, index: 17, name: "Sludge" }`, and this backup's
//! bank 2 (`USER 1`) holds `Sludge` at index 17. [solid]

use crate::{Error, Result};
use serde_json::Value as Json;

/// Magic at offset 0.
pub const MAGIC: &[u8; 4] = b"AF6L";
/// Where the zlib payload starts.
const PAYLOAD_OFFSET: usize = 0x70;
/// Refuse absurd inflate results — a corrupt stream shouldn't be able to exhaust memory.
const MAX_STREAM: usize = 64 << 20;

/// A parsed `.hxb` backup.
#[derive(Debug, Clone)]
pub struct Hxb {
    /// Container format version (1 in every file seen).
    pub version: u32,
    /// Device id, same space as a preset's `device` field (`0x0021_0001` = Helix Floor).
    pub device_id: u32,
    /// Device firmware version word.
    pub device_version: u32,
    /// Backup timestamp, seconds since the Unix epoch.
    pub timestamp: u32,
    /// The user's comment, NUL-trimmed.
    pub comment: String,
    /// Every inflated stream, in file order.
    pub streams: Vec<Vec<u8>>,
}

/// One setlist out of a backup.
#[derive(Debug, Clone)]
pub struct HxbSetlist {
    /// Bank index — the `bank` of `goto_preset`/`save_preset`.
    pub bank: usize,
    /// The device's own name for it, e.g. `"USER 1"`.
    pub name: String,
    /// Exactly 128 slots; `None` where the slot is empty.
    pub presets: Vec<Option<HxbPreset>>,
}

impl HxbSetlist {
    /// How many slots actually hold a preset.
    pub fn populated(&self) -> usize {
        self.presets.iter().filter(|p| p.is_some()).count()
    }
}

/// One preset inside a backup.
#[derive(Debug, Clone)]
pub struct HxbPreset {
    /// Index within its setlist — pairs with [`HxbSetlist::bank`] for `goto_preset`.
    pub index: usize,
    /// Display name.
    pub name: String,
    /// The raw `tone` object. **JSON, not the wire blob** — kept whole so nothing is lost, but
    /// converting it to a writable preset stream is not implemented.
    pub tone: Json,
}

impl Hxb {
    /// Parse a `.hxb` file.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < PAYLOAD_OFFSET || &bytes[0..4] != MAGIC {
            return Err(Error::Stream("not an .hxb backup (bad AF6L magic)".into()));
        }
        let u32_at = |off: usize| -> u32 {
            u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
        };
        let comment_end = bytes[0x30..0x70]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(0x40);
        Ok(Self {
            version: u32_at(0x04),
            device_id: u32_at(0x18),
            device_version: u32_at(0x1c),
            timestamp: u32_at(0x28),
            comment: String::from_utf8_lossy(&bytes[0x30..0x30 + comment_end]).into_owned(),
            streams: inflate_all(&bytes[PAYLOAD_OFFSET..]),
        })
    }

    /// Every stream that parses as JSON with the given `schema`, in file order.
    fn by_schema<'a>(&'a self, schema: &str) -> impl Iterator<Item = Json> + 'a {
        let schema = schema.to_string();
        self.streams.iter().filter_map(move |s| {
            let j: Json = serde_json::from_slice(s).ok()?;
            (j.get("schema")?.as_str()? == schema).then_some(j)
        })
    }

    /// The device's global settings (`#0`), if present.
    pub fn globals(&self) -> Option<Json> {
        self.streams
            .first()
            .and_then(|s| serde_json::from_slice(s).ok())
    }

    /// The setlists, in bank order. This ordering *is* the bank numbering.
    pub fn setlists(&self) -> Vec<HxbSetlist> {
        self.by_schema("L6Setlist")
            .enumerate()
            .map(|(bank, j)| {
                let data = j.get("data");
                let name = data
                    .and_then(|d| d.get("meta"))
                    .and_then(|m| m.get("name"))
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string();
                let presets = data
                    .and_then(|d| d.get("presets"))
                    .and_then(Json::as_array)
                    .map(|arr| {
                        arr.iter()
                            .enumerate()
                            .map(|(index, p)| {
                                // An empty slot is an object with no `tone` key.
                                let tone = p.get("tone")?;
                                Some(HxbPreset {
                                    index,
                                    name: p
                                        .get("meta")
                                        .and_then(|m| m.get("name"))
                                        .and_then(Json::as_str)
                                        .unwrap_or("")
                                        .to_string(),
                                    tone: tone.clone(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                HxbSetlist {
                    bank,
                    name,
                    presets,
                }
            })
            .collect()
    }

    /// The IR slots, as raw RIFF WAV bytes (32-bit float, 48 kHz, mono). An all-zero or absent
    /// slot is still returned — the device keeps 128 either way.
    pub fn impulse_responses(&self) -> Vec<&[u8]> {
        self.streams
            .iter()
            .filter(|s| s.starts_with(b"RIFF"))
            .map(|s| s.as_slice())
            .collect()
    }
}

/// Inflate concatenated raw zlib streams. Each stream ends where the decompressor says it does,
/// and the next begins at the following zlib header; stray padding between/after them (the Floor
/// backup ends with two NUL bytes) is skipped rather than treated as an error.
fn inflate_all(mut payload: &[u8]) -> Vec<Vec<u8>> {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut out = Vec::new();
    while !payload.is_empty() {
        // zlib streams start with 0x78 (CMF for deflate/32K window).
        if payload[0] != 0x78 {
            payload = &payload[1..];
            continue;
        }
        let mut d = Decompress::new(true);
        let mut buf = Vec::with_capacity(payload.len() * 4);
        let status = loop {
            let before = d.total_out() as usize;
            buf.resize(buf.len().max(64 * 1024) * 2, 0);
            match d.decompress(
                &payload[d.total_in() as usize..],
                &mut buf[before..],
                FlushDecompress::None,
            ) {
                Ok(Status::StreamEnd) => break Some(()),
                Ok(Status::Ok) | Ok(Status::BufError)
                    if (d.total_out() as usize) < MAX_STREAM
                        && (d.total_in() as usize) < payload.len() => {}
                _ => break None,
            }
        };
        let consumed = d.total_in() as usize;
        if status.is_none() || consumed == 0 {
            payload = &payload[1..]; // not a real stream after all — resync
            continue;
        }
        buf.truncate(d.total_out() as usize);
        out.push(buf);
        payload = &payload[consumed..];
    }
    out
}
