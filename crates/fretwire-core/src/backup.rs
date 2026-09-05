//! Backup files — a JSON envelope holding one raw preset stream per setlist slot, and (version 3)
//! the device's global settings and user IR store beside them.
//!
//! The stored unit for a preset is the **raw reassembled read-stream payload** (what
//! `read_preset_raw` returns), not the bare op-21 blob: the raw form is the one
//! `PresetStream::parse` accepts, so a backup can be inspected and validated offline; the writable
//! blob is derived at restore time (`parse → to_blob`, exactly how live structural edits already
//! do it).
//!
//! Format (version 3):
//! ```json
//! { "format": "fretwire-backup", "version": 3, "device": "HX Stomp",
//!   "setlists": [ { "bank": 0, "name": "FACTORY 1" } ],
//!   "presets": [ { "bank": 0, "index": 0, "name": "My Tone", "raw_hex": "83a6…" } ],
//!   "settings": [ { "id": 16, "type": "f32", "value": 120.0, "name": "Tempo" } ],
//!   "irs": [ { "slot": 0, "name": "412 V30", "raw_hex": "0000…" } ] }
//! ```
//! Blobs are hex-encoded — ~2× size (a full 126-preset setlist is a few MB, a full IR store two
//! more), zero new dependencies, and trivially diffable/greppable.
//!
//! **Settings are stored typed.** The device refuses a write whose type differs from what it holds
//! (`-3`), so a restore has to send a `bool` as a bool and an `f32` as an `f32`; the `type` field
//! is what makes that possible without a read first. Every id that answered is recorded, named
//! or not, because a file exists to be read later — but a restore **writes only the identified
//! ids** (`fretwire_protocol::settings::is_writable`), which is the same rule the settings panel
//! keeps. The `name` beside each is advisory, like the setlist names.
//!
//! **A presets-only file is still written as version 2**, so an older build reads an export that
//! carries nothing it would not understand. Version 3 is written when either extra section holds
//! something.
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

/// A setting's value, in the type the device holds it — the three types the op-24 read has ever
/// answered with. Kept as its own enum rather than an `rmpv::Value` so the file format names
/// exactly what it can round-trip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    F32(f32),
}

impl SettingValue {
    /// From the value an op-24 read answered. `None` for a type no setting has ever held, which
    /// the sweep records as absent rather than guessing at.
    pub fn from_rmpv(v: &fretwire_data::rmpv::Value) -> Option<SettingValue> {
        use fretwire_data::rmpv::Value;
        match v {
            Value::Boolean(b) => Some(SettingValue::Bool(*b)),
            Value::Integer(n) => n.as_i64().map(SettingValue::Int),
            Value::F32(f) => Some(SettingValue::F32(*f)),
            Value::F64(f) => Some(SettingValue::F32(*f as f32)),
            _ => None,
        }
    }

    /// The value as the op-25 write sends it — the same type it was read in.
    pub fn to_rmpv(self) -> fretwire_data::rmpv::Value {
        use fretwire_data::rmpv::Value;
        match self {
            SettingValue::Bool(b) => Value::from(b),
            SettingValue::Int(n) => Value::from(n),
            SettingValue::F32(f) => Value::F32(f),
        }
    }

    /// The `type` tag the file carries.
    pub fn type_name(self) -> &'static str {
        match self {
            SettingValue::Bool(_) => "bool",
            SettingValue::Int(_) => "int",
            SettingValue::F32(_) => "f32",
        }
    }

    /// Whether `other` holds the same type — what decides if a stored value may be written over
    /// the device's current one.
    pub fn same_type(self, other: SettingValue) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

impl std::fmt::Display for SettingValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingValue::Bool(b) => write!(f, "{b}"),
            SettingValue::Int(n) => write!(f, "{n}"),
            SettingValue::F32(x) => write!(f, "{x}"),
        }
    }
}

/// One global setting as the device answered it at backup time.
#[derive(Debug, Clone, PartialEq)]
pub struct BackupSetting {
    pub id: i64,
    pub value: SettingValue,
}

/// One user IR slot: its index, the name the device stores, and the raw sample blob (little-endian
/// `f32`, as `Session::ir_export` returns it and `Session::ir_upload` takes it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupIr {
    /// Zero-based slot, as the IR ops address it.
    pub slot: i64,
    pub name: String,
    pub blob: Vec<u8>,
}

/// One favorite, as the device holds it: its place in the list, its name, and the record op 113
/// answered — the block's model and values with its paired cab — kept as the MessagePack bytes
/// the device sent, since that is what a restore will have to send back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFavorite {
    /// Index in the favorites list (op 112's `118`).
    pub index: i64,
    pub name: String,
    /// The block's model, a `Helix.sym` index.
    pub model: i64,
    /// The paired cab's `Helix.sym` index, for an amp favorite.
    pub paired_cab: Option<i64>,
    /// The record (op 113's `64`), MessagePack-encoded.
    pub record: Vec<u8>,
}

/// One user default: the form of the model it was saved for and the record the device answered
/// (op 109), MessagePack-encoded like a favorite's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupUserDefault {
    /// The model, a `Helix.sym` index.
    pub model: i64,
    /// `None` for the model on its own; for its amp-with-cab form, the cab kind the ask named (the
    /// `Helix.sym` index of the first legacy cab or the first mic'd cab).
    pub cab_kind: Option<i64>,
    pub record: Vec<u8>,
}

/// A whole backup file: device tag + the presets it holds (any subset of any setlists), and
/// whatever of the device's settings, IRs, favorites and user defaults the sweep was asked for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Backup {
    pub device: String,
    /// The setlists this file covers, `(bank, name)`, in bank order — the names the device reported
    /// at export time. Display only; `BackupPreset::bank` is what addresses a restore. Empty on a
    /// version-1 file, which recorded no names.
    pub setlists: Vec<(i64, String)>,
    pub presets: Vec<BackupPreset>,
    /// Every global setting that answered, in id order. Empty on a presets-only export.
    pub settings: Vec<BackupSetting>,
    /// The populated user IR slots, in slot order. Empty on a presets-only export.
    pub irs: Vec<BackupIr>,
    /// The favorites, in list order. Version 4.
    pub favorites: Vec<BackupFavorite>,
    /// Every (model, form) that holds a user default. Version 4.
    pub user_defaults: Vec<BackupUserDefault>,
}

const FORMAT: &str = "fretwire-backup";
/// The version a file with favorites or user defaults is written as.
const VERSION: i64 = 4;
/// The version a file with settings or IRs but neither of the above is written as — what every
/// build since 0.5 reads.
const DEVICE_VERSION: i64 = 3;
/// The version a presets-only file is written as — the last one every released build reads.
const PRESETS_ONLY_VERSION: i64 = 2;
/// The oldest version we still read. See the module docs on why v1 needs no migration.
const MIN_VERSION: i64 = 1;

impl Backup {
    /// Whether the file carries anything beyond presets.
    pub fn is_presets_only(&self) -> bool {
        self.settings.is_empty()
            && self.irs.is_empty()
            && self.favorites.is_empty()
            && self.user_defaults.is_empty()
    }

    /// The version [`Self::to_json`] writes this backup as: the oldest one that can carry what
    /// this file holds, so a file never claims a version it does not need.
    pub fn version(&self) -> i64 {
        if self.is_presets_only() {
            PRESETS_ONLY_VERSION
        } else if self.favorites.is_empty() && self.user_defaults.is_empty() {
            DEVICE_VERSION
        } else {
            VERSION
        }
    }

    /// Serialize to JSON (pretty-printed) — version 2 for a presets-only file, 3 with settings or
    /// IRs, 4 with favorites or user defaults.
    pub fn to_json(&self) -> String {
        use fretwire_protocol::settings::by_id;
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
        let mut root = serde_json::json!({
            "format": FORMAT,
            "version": self.version(),
            "device": self.device,
            "setlists": setlists,
            "presets": presets,
        });
        if !self.is_presets_only() {
            let settings: Vec<serde_json::Value> = self
                .settings
                .iter()
                .map(|s| {
                    let value = match s.value {
                        SettingValue::Bool(b) => serde_json::Value::from(b),
                        SettingValue::Int(n) => serde_json::Value::from(n),
                        SettingValue::F32(f) => serde_json::Value::from(f),
                    };
                    let mut e = serde_json::json!({
                        "id": s.id,
                        "type": s.value.type_name(),
                        "value": value,
                    });
                    // Advisory, like the setlist names: what the id meant when the file was
                    // written, for a reader without the table.
                    if let Some(def) = by_id(s.id) {
                        e["name"] = serde_json::Value::from(def.name);
                    }
                    e
                })
                .collect();
            let irs: Vec<serde_json::Value> = self
                .irs
                .iter()
                .map(|ir| {
                    serde_json::json!({
                        "slot": ir.slot,
                        "name": ir.name,
                        "raw_hex": hex_encode(&ir.blob),
                    })
                })
                .collect();
            root["settings"] = serde_json::Value::Array(settings);
            root["irs"] = serde_json::Value::Array(irs);
        }
        if self.version() >= VERSION {
            let favorites: Vec<serde_json::Value> = self
                .favorites
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "index": f.index,
                        "name": f.name,
                        "model": f.model,
                        "paired_cab": f.paired_cab,
                        "record_hex": hex_encode(&f.record),
                    })
                })
                .collect();
            let user_defaults: Vec<serde_json::Value> = self
                .user_defaults
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "model": d.model,
                        "cab_kind": d.cab_kind,
                        "record_hex": hex_encode(&d.record),
                    })
                })
                .collect();
            root["favorites"] = serde_json::Value::Array(favorites);
            root["user_defaults"] = serde_json::Value::Array(user_defaults);
        }
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
                        Some((
                            e["bank"].as_i64()?,
                            e["name"].as_str().unwrap_or("").to_string(),
                        ))
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
        // Both optional: absent before version 3, and a v3 file may carry one without the other.
        let mut settings = Vec::new();
        if let Some(list) = root["settings"].as_array() {
            for (i, e) in list.iter().enumerate() {
                let id = e["id"]
                    .as_i64()
                    .ok_or_else(|| bad(format!("setting #{i}: missing \"id\"")))?;
                let ty = e["type"].as_str().unwrap_or("");
                let v = &e["value"];
                let value = match ty {
                    "bool" => v.as_bool().map(SettingValue::Bool),
                    "int" => v.as_i64().map(SettingValue::Int),
                    "f32" => v.as_f64().map(|f| SettingValue::F32(f as f32)),
                    _ => None,
                }
                .ok_or_else(|| {
                    bad(format!(
                        "setting {id}: type {ty:?} and value {v} do not go together"
                    ))
                })?;
                settings.push(BackupSetting { id, value });
            }
        }
        let mut irs = Vec::new();
        if let Some(list) = root["irs"].as_array() {
            for (i, e) in list.iter().enumerate() {
                let slot = e["slot"]
                    .as_i64()
                    .ok_or_else(|| bad(format!("IR #{i}: missing \"slot\"")))?;
                let name = e["name"].as_str().unwrap_or("").to_string();
                let hex = e["raw_hex"]
                    .as_str()
                    .ok_or_else(|| bad(format!("IR #{i}: missing \"raw_hex\"")))?;
                let blob = hex_decode(hex)
                    .map_err(|m| bad(format!("IR slot {slot} (\"{name}\"): {m}")))?;
                irs.push(BackupIr { slot, name, blob });
            }
        }
        // Version 4: both optional, both absent before.
        let mut favorites = Vec::new();
        if let Some(list) = root["favorites"].as_array() {
            for (i, e) in list.iter().enumerate() {
                let index = e["index"]
                    .as_i64()
                    .ok_or_else(|| bad(format!("favorite #{i}: missing \"index\"")))?;
                let name = e["name"].as_str().unwrap_or("").to_string();
                let model = e["model"]
                    .as_i64()
                    .ok_or_else(|| bad(format!("favorite #{i}: missing \"model\"")))?;
                let hex = e["record_hex"]
                    .as_str()
                    .ok_or_else(|| bad(format!("favorite #{i}: missing \"record_hex\"")))?;
                let record = hex_decode(hex)
                    .map_err(|m| bad(format!("favorite {index} (\"{name}\"): {m}")))?;
                favorites.push(BackupFavorite {
                    index,
                    name,
                    model,
                    paired_cab: e["paired_cab"].as_i64(),
                    record,
                });
            }
        }
        let mut user_defaults = Vec::new();
        if let Some(list) = root["user_defaults"].as_array() {
            for (i, e) in list.iter().enumerate() {
                let model = e["model"]
                    .as_i64()
                    .ok_or_else(|| bad(format!("user default #{i}: missing \"model\"")))?;
                let hex = e["record_hex"]
                    .as_str()
                    .ok_or_else(|| bad(format!("user default #{i}: missing \"record_hex\"")))?;
                let record = hex_decode(hex)
                    .map_err(|m| bad(format!("user default for model {model}: {m}")))?;
                user_defaults.push(BackupUserDefault {
                    model,
                    cab_kind: e["cab_kind"].as_i64(),
                    record,
                });
            }
        }
        Ok(Backup {
            device,
            setlists,
            presets,
            settings,
            favorites,
            user_defaults,
            irs,
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

/// Which parts of a backup a restore writes. Each is only meaningful where the file holds that
/// part; a presets-only file with `irs` set restores no IRs and reports none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreParts {
    pub presets: bool,
    pub irs: bool,
    pub settings: bool,
}

impl RestoreParts {
    /// Everything the file holds.
    pub const ALL: RestoreParts = RestoreParts {
        presets: true,
        irs: true,
        settings: true,
    };
}

/// What one part of a device restore did with each item — the report is a list of these, one per
/// preset, IR and setting the file held, so a caller can say exactly what changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// Written to the device.
    Written,
    /// Already held this value, so nothing was written — a restore onto a pedal that was not wiped
    /// leaves the parts that still match alone.
    Unchanged,
    /// Deliberately not written, with the reason: a setting id nobody has identified, or a part
    /// the caller left out.
    Skipped(String),
    /// The device refused it or the write failed, with the error. The restore goes on to the next
    /// item; one bad slot should not cost the rest.
    Failed(String),
}

/// What a device restore did, item by item. `presets` is keyed `(bank, index)`, `irs` by slot,
/// `settings` by id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub presets: Vec<((i64, i64), RestoreOutcome)>,
    pub irs: Vec<(i64, RestoreOutcome)>,
    pub settings: Vec<(i64, RestoreOutcome)>,
}

impl RestoreReport {
    /// How many of `items` ended as `Written`.
    pub fn written<K>(items: &[(K, RestoreOutcome)]) -> usize {
        items
            .iter()
            .filter(|(_, o)| *o == RestoreOutcome::Written)
            .count()
    }

    /// How many of `items` ended as `Unchanged`.
    pub fn unchanged<K>(items: &[(K, RestoreOutcome)]) -> usize {
        items
            .iter()
            .filter(|(_, o)| *o == RestoreOutcome::Unchanged)
            .count()
    }

    /// Every failure across the three sections, as `(what, error)` lines.
    pub fn failures(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for ((bank, index), o) in &self.presets {
            if let RestoreOutcome::Failed(e) = o {
                out.push((format!("preset {bank}:{index}"), e.clone()));
            }
        }
        for (slot, o) in &self.irs {
            if let RestoreOutcome::Failed(e) = o {
                out.push((format!("IR slot {slot}"), e.clone()));
            }
        }
        for (id, o) in &self.settings {
            if let RestoreOutcome::Failed(e) = o {
                out.push((format!("setting {id}"), e.clone()));
            }
        }
        out
    }
}

fn bad(msg: String) -> Error {
    Error::Backup(msg)
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
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
            ..Default::default()
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
        let too_new = r#"{"format":"fretwire-backup","version":5,"device":"x","presets":[]}"#;
        assert!(Backup::from_json(too_new).is_err());
        let bad_hex = r#"{"format":"fretwire-backup","version":2,"device":"x",
            "presets":[{"bank":0,"index":0,"name":"a","raw_hex":"zz"}]}"#;
        assert!(Backup::from_json(bad_hex).is_err());
    }

    /// Favorites and user defaults make a file version 4; a file with settings and IRs but
    /// neither stays version 3, so a 0.5 build still reads a backup that did not ask for them.
    /// The records round-trip as the bytes the device sent.
    #[test]
    fn version_4_carries_favorites_and_user_defaults() {
        let mut b = Backup {
            device: "HX Stomp".into(),
            settings: vec![BackupSetting {
                id: 27,
                value: SettingValue::Bool(true),
            }],
            ..Default::default()
        };
        assert_eq!(b.version(), 3);
        b.favorites.push(BackupFavorite {
            index: 0,
            name: "US Princess".into(),
            model: 591,
            paired_cab: Some(709),
            record: vec![0x81, 0x13, 0x06],
        });
        b.favorites.push(BackupFavorite {
            index: 1,
            name: "Dynamic Plate".into(),
            model: 636,
            paired_cab: None,
            record: vec![0x81, 0x13, 0x07],
        });
        b.user_defaults.push(BackupUserDefault {
            model: 591,
            cab_kind: Some(687),
            record: vec![0x81, 0x13, 0x08],
        });
        assert_eq!(b.version(), 4);
        let json = b.to_json();
        assert!(json.contains("\"favorites\"") && json.contains("\"user_defaults\""));
        let back = Backup::from_json(&json).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.favorites[1].paired_cab, None);
        // Favorites alone are enough for version 4 — no settings needed.
        let only = Backup {
            device: "HX Stomp".into(),
            favorites: b.favorites.clone(),
            ..Default::default()
        };
        assert_eq!(only.version(), 4);
        assert!(!only.is_presets_only());
        assert_eq!(Backup::from_json(&only.to_json()).unwrap(), only);
    }

    /// A presets-only file is still version 2 — an older build reads it — and a file with either
    /// extra section is version 3, with the settings typed so a restore can send them back as
    /// the device holds them.
    #[test]
    fn version_3_carries_settings_and_irs_and_round_trips() {
        let mut b = Backup {
            device: "HX Stomp".into(),
            presets: vec![BackupPreset {
                bank: 0,
                index: 3,
                name: "A".into(),
                raw: vec![1, 2],
            }],
            ..Default::default()
        };
        assert_eq!(b.version(), 2);
        assert!(b.to_json().contains("\"version\": 2"));
        assert!(!b.to_json().contains("\"settings\""));

        b.settings = vec![
            BackupSetting {
                id: 10,
                value: SettingValue::Bool(true),
            },
            BackupSetting {
                id: 16,
                value: SettingValue::F32(121.5),
            },
            BackupSetting {
                id: 134,
                value: SettingValue::Int(2),
            },
            // Unidentified ids are recorded too — a file is for reading later.
            BackupSetting {
                id: 210,
                value: SettingValue::Int(7),
            },
        ];
        b.irs = vec![BackupIr {
            slot: 5,
            name: "412 V30".into(),
            blob: vec![0, 0, 128, 63],
        }];
        assert_eq!(b.version(), 3);
        let json = b.to_json();
        assert!(json.contains("\"version\": 3"));
        // The advisory name rides along where the table has one, and not where it doesn't.
        assert!(json.contains("\"name\": \"BPM\""));
        let back = Backup::from_json(&json).unwrap();
        assert_eq!(b, back);
        assert_eq!(back.settings[1].value, SettingValue::F32(121.5));
        assert!(back.settings[0].value.same_type(SettingValue::Bool(false)));
        assert!(!back.settings[0].value.same_type(SettingValue::Int(0)));
    }

    /// A v3 file whose typed value does not match its tag is refused rather than coerced: a
    /// wrong-typed write is what the device refuses, so the file must not be able to ask for one.
    #[test]
    fn mistyped_setting_is_refused() {
        let bad = r#"{"format":"fretwire-backup","version":3,"device":"x","presets":[],
            "settings":[{"id":16,"type":"f32","value":true}],"irs":[]}"#;
        assert!(Backup::from_json(bad).is_err());
        let unknown = r#"{"format":"fretwire-backup","version":3,"device":"x","presets":[],
            "settings":[{"id":16,"type":"string","value":"x"}],"irs":[]}"#;
        assert!(Backup::from_json(unknown).is_err());
    }

    #[test]
    fn restore_report_counts_and_failures() {
        let r = RestoreReport {
            presets: vec![
                ((0, 1), RestoreOutcome::Written),
                ((0, 2), RestoreOutcome::Unchanged),
                ((0, 3), RestoreOutcome::Failed("stalled".into())),
            ],
            irs: vec![(4, RestoreOutcome::Written)],
            settings: vec![
                (16, RestoreOutcome::Written),
                (210, RestoreOutcome::Skipped("unidentified".into())),
            ],
        };
        assert_eq!(RestoreReport::written(&r.presets), 1);
        assert_eq!(RestoreReport::written(&r.irs), 1);
        assert_eq!(RestoreReport::written(&r.settings), 1);
        assert_eq!(
            r.failures(),
            vec![("preset 0:3".to_string(), "stalled".to_string())]
        );
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
