//! Parser for `.hlx` preset files (`schema: "L6Preset"`).
//!
//! The outer envelope is stable and typed here. The `tone` tree is highly dynamic
//! (`dsp0`/`dsp1` signal paths, `global`, `snapshot0..7`, and effect blocks keyed by slot)
//! and its block parameter maps mix `@`-prefixed structural keys with model parameters.
//! We keep `tone` as raw JSON so that parse → serialize is **lossless**, which matters
//! because edited presets will eventually be re-encoded and sent back to the device.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub version: i64,
    pub data: PresetData,
    /// e.g. `"L6Preset"`. Absent in some default fixtures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetData {
    pub meta: Meta,
    /// Device id (e.g. HX Stomp `2162694` = `0x210006`). Encoding TBD.
    pub device: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_version: Option<i64>,
    /// `dsp0`, `dsp1`, `global`, `snapshot0..7`, and effect block slots. Kept raw for
    /// lossless round-trip; typed accessors live on [`Preset`].
    pub tone: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub name: String,
    #[serde(default)]
    pub application: Option<String>,
    #[serde(default)]
    pub build_sha: Option<String>,
    #[serde(default)]
    pub modifieddate: Option<i64>,
    #[serde(default)]
    pub appversion: Option<i64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Preset {
    pub fn from_slice(bytes: &[u8]) -> crate::Result<Self> {
        // Through `Value` first for duplicate-key tolerance (see `ModelFile::from_slice`).
        let value: Value = serde_json::from_slice(bytes)?;
        Ok(serde_json::from_value(value)?)
    }

    pub fn from_str(s: &str) -> crate::Result<Self> {
        Self::from_slice(s.as_bytes())
    }

    /// Preset display name (`data.meta.name`).
    pub fn name(&self) -> &str {
        &self.data.meta.name
    }

    /// Iterate the signal-path keys present in `tone` (`dsp0`, `dsp1`, ...).
    pub fn dsp_paths(&self) -> impl Iterator<Item = &String> {
        self.data
            .tone
            .keys()
            .filter(|k| k.starts_with("dsp"))
    }
}
