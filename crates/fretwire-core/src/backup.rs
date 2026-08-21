//! Preset backup files — a JSON envelope holding one raw preset stream per setlist slot.
//!
//! The stored unit is the **raw reassembled read-stream payload** (what `read_preset_raw`
//! returns), not the bare op-21 blob: the raw form is the one `PresetStream::parse` accepts, so a
//! backup can be inspected and validated offline; the writable blob is derived at restore time
//! (`parse → to_blob`, exactly how live structural edits already do it).
//!
//! Format (version 2):
//! ```json
//! { "format": "fretwire-backup", "version": 2, "device": "Helix Floor",
//!   "setlists": [ { "bank": 0, "name": "FACTORY 1" } ],
//!   "presets": [ { "bank": 0, "index": 0, "name": "My Tone", "raw_hex": "83a6…" } ] }
//! ```
//! Blobs are hex-encoded — ~2× size (a full 126-preset setlist is a few MB), zero new
//! dependencies, and trivially diffable/greppable.
//!
//! **Version 1 files still read.** They predate multi-setlist export and carry no `bank`, which is
//! not a gap: every one of them was written by a sweep that walked bank 0 and nothing else, so
//! reading them as bank 0 is exact, not a default. The `setlists` array is advisory — the names the
//! device gave at export time, for display — and v1 files simply have none.

use crate::{Error, Result};

/// One backed-up preset: which setlist it came from, its slot in that setlist, its stored name,
/// and the raw read-stream payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPreset {
    /// Setlist this came from (as `goto_preset`/`list_presets_in` take). `0` on a device with one
    /// setlist, and on every version-1 file.
    pub bank: i64,
    /// Slot **within that setlist** (as `goto`/`list_presets_in` use) — not a global preset number.
    pub index: i64,
    /// Preset name at backup time (from the op-23 identity read).
    pub name: String,
    /// Raw reassembled preset stream (parseable with `PresetStream::parse`).
    pub raw: Vec<u8>,
}

/// A whole backup file: device tag + the presets it holds (any subset of any setlists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub device: String,
    /// The setlists this file covers, `(bank, name)`, in bank order — the names the device reported
    /// at export time. Display only; `BackupPreset::bank` is what addresses a restore. Empty on a
    /// version-1 file, which recorded no names.
    pub setlists: Vec<(i64, String)>,
    pub presets: Vec<BackupPreset>,
}

const FORMAT: &str = "fretwire-backup";
const VERSION: i64 = 2;
/// The oldest version we still read. See the module docs on why v1 needs no migration.
const MIN_VERSION: i64 = 1;

impl Backup {
    /// Serialize to the current JSON format (pretty-printed).
    pub fn to_json(&self) -> String {
        let presets: Vec<serde_json::Value> = self
            .presets
            .iter()
            .map(|p| {
                serde_json::json!({
                    "bank": p.bank,
                    "index": p.index,
                    "name": p.name,
                    "raw_hex": hex_encode(&p.raw),
                })
            })
            .collect();
        let setlists: Vec<serde_json::Value> = self
            .setlists
            .iter()
            .map(|(bank, name)| serde_json::json!({ "bank": bank, "name": name }))
            .collect();
        let root = serde_json::json!({
            "format": FORMAT,
            "version": VERSION,
            "device": self.device,
            "setlists": setlists,
            "presets": presets,
        });
        serde_json::to_string_pretty(&root).expect("json of plain values")
    }

    /// Parse a backup file, validating the format tag, version, and hex payloads.
    pub fn from_json(s: &str) -> Result<Backup> {
        let root: serde_json::Value =
            serde_json::from_str(s).map_err(|e| bad(format!("not JSON: {e}")))?;
        if root["format"].as_str() != Some(FORMAT) {
            return Err(bad(
                "missing/wrong \"format\" tag — not a fretwire-backup file".into(),
            ));
        }
        let version = root["version"].as_i64().unwrap_or(0);
        if !(MIN_VERSION..=VERSION).contains(&version) {
            return Err(bad(format!(
                "unsupported backup version {version} (this build reads {MIN_VERSION}..={VERSION})"
            )));
        }
        let device = root["device"].as_str().unwrap_or("unknown").to_string();
        let setlists = root["setlists"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some((e["bank"].as_i64()?, e["name"].as_str().unwrap_or("").to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let entries = root["presets"]
            .as_array()
            .ok_or_else(|| bad("missing \"presets\" array".into()))?;
        let mut presets = Vec::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            let index = e["index"]
                .as_i64()
                .ok_or_else(|| bad(format!("preset #{i}: missing \"index\"")))?;
            // Absent in every version-1 file, where it is exactly 0 rather than unknown.
            let bank = e["bank"].as_i64().unwrap_or(0);
            let name = e["name"].as_str().unwrap_or("").to_string();
            let hex = e["raw_hex"]
                .as_str()
                .ok_or_else(|| bad(format!("preset #{i}: missing \"raw_hex\"")))?;
            let raw = hex_decode(hex).map_err(|m| bad(format!("preset #{i} (\"{name}\"): {m}")))?;
            presets.push(BackupPreset {
                bank,
                index,
                name,
                raw,
            });
        }
        Ok(Backup {
            device,
            setlists,
            presets,
        })
    }

    /// The entry stored for slot `index` of setlist `bank`, if the file has one.
    pub fn preset(&self, bank: i64, index: i64) -> Option<&BackupPreset> {
        self.presets
            .iter()
            .find(|p| p.bank == bank && p.index == index)
    }

    /// The name this file recorded for `bank`, if it recorded one (version 2+).
    pub fn setlist_name(&self, bank: i64) -> Option<&str> {
        self.setlists
            .iter()
            .find(|(b, _)| *b == bank)
            .map(|(_, n)| n.as_str())
    }

    /// Every setlist the file actually holds presets for, in bank order.
    pub fn banks(&self) -> Vec<i64> {
        let mut banks: Vec<i64> = self.presets.iter().map(|p| p.bank).collect();
        banks.sort_unstable();
        banks.dedup();
        banks
    }
}

fn bad(msg: String) -> Error {
    Error::Backup(msg)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        write!(s, "{b:02x}").expect("write to String");
    }
    s
}

fn hex_decode(s: &str) -> std::result::Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("odd-length hex".into());
    }
    (0..s.len() / 2)
        .map(|i| {
            u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| format!("bad hex at byte {i}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let b = Backup {
            device: "Helix Floor".into(),
            setlists: vec![(0, "FACTORY 1".into()), (2, "USER 1".into())],
            presets: vec![
                BackupPreset {
                    bank: 0,
                    index: 0,
                    name: "Lead".into(),
                    raw: vec![0x83, 0xa6, 0x00, 0xff],
                },
                BackupPreset {
                    bank: 2,
                    index: 0,
                    name: "".into(),
                    raw: vec![],
                },
                BackupPreset {
                    bank: 2,
                    index: 125,
                    name: "Rhythm".into(),
                    raw: vec![0x01],
                },
            ],
        };
        let json = b.to_json();
        let back = Backup::from_json(&json).unwrap();
        assert_eq!(b, back);
        // Slot 0 exists in two setlists and they are different presets — the whole point of v2.
        assert_eq!(back.preset(0, 0).unwrap().name, "Lead");
        assert_eq!(back.preset(2, 0).unwrap().name, "");
        assert!(back.preset(1, 0).is_none());
        assert_eq!(back.banks(), vec![0, 2]);
        assert_eq!(back.setlist_name(2), Some("USER 1"));
        assert_eq!(back.setlist_name(1), None);
    }

    /// Version-1 files are still read, and their presets are bank 0 — not as a default, but because
    /// the only sweep that ever wrote one walked bank 0 and nothing else.
    #[test]
    fn version_1_files_still_read_as_bank_zero() {
        let v1 = r#"{"format":"fretwire-backup","version":1,"device":"HX Stomp",
            "presets":[{"index":7,"name":"Old","raw_hex":"0a0b"}]}"#;
        let b = Backup::from_json(v1).unwrap();
        assert_eq!(b.device, "HX Stomp");
        assert!(b.setlists.is_empty(), "v1 recorded no setlist names");
        assert_eq!(b.banks(), vec![0]);
        let e = b.preset(0, 7).unwrap();
        assert_eq!((e.bank, e.index, e.name.as_str()), (0, 7, "Old"));
        assert_eq!(e.raw, vec![0x0a, 0x0b]);
    }

    #[test]
    fn rejects_foreign_files() {
        assert!(Backup::from_json("{}").is_err());
        assert!(Backup::from_json("not json").is_err());
        let too_new = r#"{"format":"fretwire-backup","version":3,"device":"x","presets":[]}"#;
        assert!(Backup::from_json(too_new).is_err());
        let bad_hex = r#"{"format":"fretwire-backup","version":2,"device":"x",
            "presets":[{"bank":0,"index":0,"name":"a","raw_hex":"zz"}]}"#;
        assert!(Backup::from_json(bad_hex).is_err());
    }

    #[test]
    fn hex_helpers() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xf0, 0xff]), "000ff0ff");
        assert_eq!(
            hex_decode("000ff0ff").unwrap(),
            vec![0x00, 0x0f, 0xf0, 0xff]
        );
        assert!(hex_decode("abc").is_err());
    }
}
