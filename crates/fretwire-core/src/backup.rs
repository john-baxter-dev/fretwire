//! Preset backup files — a JSON envelope holding one raw preset stream per setlist slot.
//!
//! The stored unit is the **raw reassembled read-stream payload** (what `read_preset_raw`
//! returns), not the bare op-21 blob: the raw form is the one `PresetStream::parse` accepts, so a
//! backup can be inspected and validated offline; the writable blob is derived at restore time
//! (`parse → to_blob`, exactly how live structural edits already do it).
//!
//! Format (version 1):
//! ```json
//! { "format": "fretwire-backup", "version": 1, "device": "HX Stomp",
//!   "presets": [ { "index": 0, "name": "My Tone", "raw_hex": "83a6…" } ] }
//! ```
//! Blobs are hex-encoded — ~2× size (a full 126-preset setlist is a few MB), zero new
//! dependencies, and trivially diffable/greppable.

use crate::{Error, Result};

/// One backed-up preset: its setlist slot, stored name, and raw read-stream payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPreset {
    /// Flat setlist index (as `goto`/`list_presets` use).
    pub index: i64,
    /// Preset name at backup time (from the op-23 identity read).
    pub name: String,
    /// Raw reassembled preset stream (parseable with `PresetStream::parse`).
    pub raw: Vec<u8>,
}

/// A whole backup file: device tag + the presets it holds (any subset of the setlist).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backup {
    pub device: String,
    pub presets: Vec<BackupPreset>,
}

const FORMAT: &str = "fretwire-backup";
const VERSION: i64 = 1;

impl Backup {
    /// Serialize to the version-1 JSON format (pretty-printed).
    pub fn to_json(&self) -> String {
        let presets: Vec<serde_json::Value> = self
            .presets
            .iter()
            .map(|p| {
                serde_json::json!({
                    "index": p.index,
                    "name": p.name,
                    "raw_hex": hex_encode(&p.raw),
                })
            })
            .collect();
        let root = serde_json::json!({
            "format": FORMAT,
            "version": VERSION,
            "device": self.device,
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
        if version != VERSION {
            return Err(bad(format!(
                "unsupported backup version {version} (expected {VERSION})"
            )));
        }
        let device = root["device"].as_str().unwrap_or("unknown").to_string();
        let entries = root["presets"]
            .as_array()
            .ok_or_else(|| bad("missing \"presets\" array".into()))?;
        let mut presets = Vec::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            let index = e["index"]
                .as_i64()
                .ok_or_else(|| bad(format!("preset #{i}: missing \"index\"")))?;
            let name = e["name"].as_str().unwrap_or("").to_string();
            let hex = e["raw_hex"]
                .as_str()
                .ok_or_else(|| bad(format!("preset #{i}: missing \"raw_hex\"")))?;
            let raw = hex_decode(hex).map_err(|m| bad(format!("preset #{i} (\"{name}\"): {m}")))?;
            presets.push(BackupPreset { index, name, raw });
        }
        Ok(Backup { device, presets })
    }

    /// The entry stored for setlist slot `index`, if the file has one.
    pub fn preset(&self, index: i64) -> Option<&BackupPreset> {
        self.presets.iter().find(|p| p.index == index)
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
            device: "HX Stomp".into(),
            presets: vec![
                BackupPreset {
                    index: 0,
                    name: "Lead".into(),
                    raw: vec![0x83, 0xa6, 0x00, 0xff],
                },
                BackupPreset {
                    index: 125,
                    name: "".into(),
                    raw: vec![],
                },
            ],
        };
        let json = b.to_json();
        let back = Backup::from_json(&json).unwrap();
        assert_eq!(b, back);
        assert_eq!(back.preset(125).unwrap().name, "");
        assert!(back.preset(1).is_none());
    }

    #[test]
    fn rejects_foreign_files() {
        assert!(Backup::from_json("{}").is_err());
        assert!(Backup::from_json("not json").is_err());
        let wrong_version = r#"{"format":"fretwire-backup","version":2,"device":"x","presets":[]}"#;
        assert!(Backup::from_json(wrong_version).is_err());
        let bad_hex = r#"{"format":"fretwire-backup","version":1,"device":"x",
            "presets":[{"index":0,"name":"a","raw_hex":"zz"}]}"#;
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
