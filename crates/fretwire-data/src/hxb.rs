//! The `.hxb` / `.pgb` **device backup** container written by HX Edit and POD Go Edit.
//!
//! One format under two extensions — the whole difference between a Floor's `.hxb` and a POD Go's
//! `.pgb` is which sections each writes. First decoded from a Helix Floor backup as a fixed header
//! plus concatenated zlib streams; a POD Go backup whose payload did *not* start at the Floor's
//! fixed offset forced a second look, which found the real structure: the container is a **tagged
//! archive with an index table at the end**. [solid — 2026-08-28, issue #15]
//!
//! ```text
//! 0x00  "AF6L"     magic
//! 0x04  u32        version (1)
//! 0x08  u32        offset of the section table
//! 0x10  u64        section count
//! 0x18  u32        device id  (0x210001 = Helix Floor, 0x210007 = POD Go — a preset's `device`)
//! 0x1c  u32        device version (matches the identity reply's, e.g. Floor 0x3800000 = fw 3.80)
//! 0x20  u32        firmware build sha, as an integer whose hex digits are the sha — the Floor's
//!                  0x07d01f5e ↔ its build `7d01f5e`, the POD Go's 0x6e984472 ↔ `v2.01-19-g6e98447`
//! 0x28  u32        unix timestamp
//! 0x30  ...        section data — every table entry points back into this region
//! ```
//!
//! Each table entry is 36 bytes: a 4-byte tag **stored reversed** (`"CSED"` on disk = `DESC`),
//! u64 data offset, u64 stored length, u32 compressed flag (1 = one zlib stream), u64 inflated
//! length, 4 pad bytes. `table offset + 36 × count` lands exactly on end-of-file in both real
//! backups we hold. Tags seen:
//!
//! | tag | holds |
//! |---|---|
//! | `HXDI` / `PGDI` | the fixed header fields themselves (offset 0x18, length 0x18) |
//! | `DESC` | the user's comment, raw text (HX Edit; POD Go Edit writes none) |
//! | `SLNM` | the setlist names, NUL-terminated, raw |
//! | `GLOB` | device globals, JSON |
//! | `I000`… | IR slots, **hex**-numbered, RIFF WAV — the Floor writes all 128 (`I000`–`I07F`), the POD Go only the populated ones |
//! | `UMDS` | the `L6UMDArchive` model-usage table, JSON |
//! | `SL00`… | the setlists, `L6Setlist` JSON — 8 on a Floor, 2 on a POD Go (`Factory`, `User`), 128 slots each |
//!
//! That is every tag the two real backups we hold carry. Neither donor had saved a **Favorite**
//! or a **User Default** (issue #5, 2026-09-04), and nothing here holds either: `GLOB` has no
//! such key and `UMDS` is a bare manifest — one entry per model the firmware knows (the POD Go's
//! 571 cover all 540 catalog models plus 31 more), with no parameter payload. Where HX Edit puts
//! them in a backup is unknown, so [`Hxb::sections`] keeps the whole table, [`HxbSection::known_as`]
//! says which tags this reading covers, and `show-backup --sections` prints both — a tag we have
//! never seen is the first thing to look for in a backup from a pedal that has them.
//!
//! A file without a valid table (we synthesize such in tests; no real one has been seen) falls
//! back to the original reading: comment at 0x30..0x70, then a scan for back-to-back zlib streams.
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
/// Where the zlib payload starts in a file with no section table (the legacy fallback reading).
const PAYLOAD_OFFSET: usize = 0x70;
/// A section-table entry on disk.
const TABLE_ENTRY: usize = 36;
/// Refuse absurd inflate results — a corrupt stream shouldn't be able to exhaust memory.
const MAX_STREAM: usize = 64 << 20;

/// A parsed `.hxb`/`.pgb` backup.
#[derive(Debug, Clone)]
pub struct Hxb {
    /// Container format version (1 in every file seen).
    pub version: u32,
    /// Device id, same space as a preset's `device` field (`0x0021_0001` = Helix Floor,
    /// `0x0021_0007` = POD Go).
    pub device_id: u32,
    /// Device firmware version word.
    pub device_version: u32,
    /// Backup timestamp, seconds since the Unix epoch.
    pub timestamp: u32,
    /// The user's comment (the `DESC` section), NUL-trimmed. POD Go Edit writes none.
    pub comment: String,
    /// The `SLNM` section: the setlist names as the editor wrote them, in bank order. Redundant
    /// with each setlist's own JSON `meta.name` in every file seen, but cheap to read — it needs
    /// no inflation. Empty when the file has no section table.
    pub setlist_names: Vec<String>,
    /// Every inflated stream, in file order.
    pub streams: Vec<Vec<u8>>,
    /// The section table as the container describes it, in file order — every entry, whether or
    /// not this parser interprets it. Empty for a file without a valid table.
    pub sections: Vec<HxbSection>,
}

/// One entry of the section table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HxbSection {
    /// The tag as read (`DESC`, `SL00`, …) — the on-disk bytes are stored reversed.
    pub tag: String,
    /// Where the section's bytes start in the file.
    pub offset: u64,
    /// How many bytes it occupies there.
    pub stored_len: u64,
    /// Whether those bytes are one zlib stream.
    pub compressed: bool,
    /// The length the table declares for the inflated data (equals `stored_len` when raw).
    pub inflated_len: u64,
}

impl HxbSection {
    /// What this reading of the format understands the tag to hold, or `None` for a tag that no
    /// backup we have seen carries — the thing to report.
    pub fn known_as(&self) -> Option<&'static str> {
        let t = self.tag.as_bytes();
        Some(match t {
            b"HXDI" | b"PGDI" => "header fields (device id, firmware, timestamp)",
            b"DESC" => "user comment",
            b"SLNM" => "setlist names",
            b"GLOB" => "global settings (JSON)",
            b"UMDS" => "model table (L6UMDArchive JSON)",
            [b'I', rest @ ..] if rest.iter().all(u8::is_ascii_hexdigit) => "IR slot (RIFF WAV)",
            [b'S', b'L', rest @ ..] if rest.iter().all(u8::is_ascii_digit) => {
                "setlist (L6Setlist JSON)"
            }
            _ => return None,
        })
    }
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
    /// Parse a `.hxb`/`.pgb` file.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < PAYLOAD_OFFSET || &bytes[0..4] != MAGIC {
            return Err(Error::Stream(
                "not an .hxb/.pgb backup (bad AF6L magic)".into(),
            ));
        }
        let u32_at = |off: usize| -> u32 {
            u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
        };
        let mut hxb = Self {
            version: u32_at(0x04),
            device_id: u32_at(0x18),
            device_version: u32_at(0x1c),
            timestamp: u32_at(0x28),
            comment: String::new(),
            setlist_names: Vec::new(),
            streams: Vec::new(),
            sections: Vec::new(),
        };
        if let Some(sections) = section_table(bytes) {
            for s in sections {
                hxb.sections.push(HxbSection {
                    tag: String::from_utf8_lossy(&s.tag).into_owned(),
                    offset: s.offset,
                    stored_len: s.data.len() as u64,
                    compressed: s.compressed,
                    inflated_len: s.inflated_len,
                });
                match (&s.tag, s.compressed) {
                    (b"DESC", false) => {
                        hxb.comment = String::from_utf8_lossy(s.data)
                            .trim_end_matches('\0')
                            .into();
                    }
                    (b"SLNM", false) => {
                        hxb.setlist_names = s
                            .data
                            .split(|&b| b == 0)
                            .filter(|n| !n.is_empty())
                            .map(|n| String::from_utf8_lossy(n).into_owned())
                            .collect();
                    }
                    // GLOB, Innn, UMDS, SLnn — the payload proper. A section that fails to
                    // inflate is dropped, matching what the legacy scan did with bytes it could
                    // not read.
                    (_, true) => hxb.streams.extend(inflate_all(s.data)),
                    // HXDI/PGDI points back at the fixed header fields already read above.
                    (_, false) => {}
                }
            }
        } else {
            // No valid table: the original fixed-layout reading.
            let comment_end = bytes[0x30..0x70]
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(0x40);
            hxb.comment = String::from_utf8_lossy(&bytes[0x30..0x30 + comment_end]).into_owned();
            hxb.streams = inflate_all(&bytes[PAYLOAD_OFFSET..]);
        }
        Ok(hxb)
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

    /// The IR slots, as raw RIFF WAV bytes (32-bit float, 48 kHz, mono). What this holds is what
    /// the editor wrote: HX Edit stores all 128 slots, empty or not; POD Go Edit stores only the
    /// populated ones.
    pub fn impulse_responses(&self) -> Vec<&[u8]> {
        self.streams
            .iter()
            .filter(|s| s.starts_with(b"RIFF"))
            .map(|s| s.as_slice())
            .collect()
    }

    /// The sections this reading of the format does not cover. Anything here is data the
    /// container holds that fretwire silently ignores — see the module docs.
    pub fn unknown_sections(&self) -> Vec<&HxbSection> {
        self.sections
            .iter()
            .filter(|s| s.known_as().is_none())
            .collect()
    }
}

/// One entry out of the section table.
struct Section<'a> {
    /// The on-disk byte-reversed tag, turned back around (`"CSED"` → `DESC`).
    tag: [u8; 4],
    offset: u64,
    data: &'a [u8],
    compressed: bool,
    inflated_len: u64,
}

/// The section table, if this file carries a valid one, in file order.
///
/// Validation is strict — the table offset (header 0x08) plus 36 bytes per entry (header 0x10)
/// must land exactly on end-of-file, and every entry must point inside the region before the
/// table — because a wrong guess here silently misreads the whole file, and the legacy scan
/// below is a working answer for anything that doesn't match.
fn section_table(bytes: &[u8]) -> Option<Vec<Section<'_>>> {
    let table = u32::from_le_bytes(bytes[0x08..0x0c].try_into().unwrap()) as usize;
    let count = u64::from_le_bytes(bytes[0x10..0x18].try_into().unwrap()) as usize;
    if table < 0x30
        || count == 0
        || table.checked_add(count.checked_mul(TABLE_ENTRY)?)? != bytes.len()
    {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for entry in bytes[table..].as_chunks::<TABLE_ENTRY>().0 {
        let tag = [entry[3], entry[2], entry[1], entry[0]];
        let off = u64::from_le_bytes(entry[4..12].try_into().unwrap()) as usize;
        let len = u64::from_le_bytes(entry[12..20].try_into().unwrap()) as usize;
        let compressed = u32::from_le_bytes(entry[20..24].try_into().unwrap()) != 0;
        let inflated_len = u64::from_le_bytes(entry[24..32].try_into().unwrap());
        if off.checked_add(len)? > table || !tag.iter().all(|b| b.is_ascii_graphic()) {
            return None;
        }
        out.push(Section {
            tag,
            offset: off as u64,
            data: &bytes[off..off + len],
            compressed,
            inflated_len,
        });
    }
    Some(out)
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
