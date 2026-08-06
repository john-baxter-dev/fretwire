//! Parser for the `*.models` files (`amp.models`, `delay.models`, ...).
//!
//! Each file is a top-level JSON array of [`Model`] objects. A model lists its parameters
//! with value ranges, defaults, and an `assign` index that we expect to correlate with the
//! on-wire parameter address (to be confirmed against USB captures).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::str::FromStr;

/// A whole `*.models` file: a flat list of models.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelFile {
    pub models: Vec<Model>,
}

impl ModelFile {
    pub fn from_slice(bytes: &[u8]) -> crate::Result<Self> {
        // Go through `Value` first: some shipped files contain duplicate keys (e.g. a
        // stray second `assign` in distortion.models). `Value`'s map is last-wins and
        // does not error, whereas deserializing straight into a struct rejects duplicates.
        let value: Value = serde_json::from_slice(bytes)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Find a model by its `symbolicID` (e.g. `"HD2_AmpGermanMahadeva"`).
    pub fn get(&self, symbolic_id: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.symbolic_id == symbolic_id)
    }
}

impl FromStr for ModelFile {
    type Err = crate::Error;

    fn from_str(s: &str) -> crate::Result<Self> {
        Self::from_slice(s.as_bytes())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    #[serde(rename = "symbolicID")]
    pub symbolic_id: String,
    /// Display name. Absent on internal pseudo-models such as `@global_params`.
    #[serde(default)]
    pub name: Option<String>,
    /// Category id (cross-references `HX_ModelCatalog.json`).
    #[serde(default)]
    pub category: Option<i64>,
    /// Suggested paired cab for amp models, if any.
    #[serde(default)]
    pub cablink: Option<String>,
    #[serde(default)]
    pub ircablink: Option<String>,
    /// DSP load (% of the device's DSP budget) for the **mono** variant. Used by the "% DSP" meter.
    #[serde(default)]
    pub load: Option<f64>,
    /// DSP load for the **stereo** variant, when the model has a distinct stereo cost.
    #[serde(default)]
    pub load_stereo: Option<f64>,
    #[serde(default)]
    pub params: Vec<Param>,
    /// Any fields we have not modeled yet — kept so we never silently drop data.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    #[serde(rename = "symbolicID")]
    pub symbolic_id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Numeric value-type tag (meaning TBD; observed values include 1). Kept raw.
    #[serde(rename = "valueType", default)]
    pub value_type: Option<i64>,
    /// e.g. `"generic_knob"`. Drives which control widget the editor renders.
    #[serde(rename = "displayType", default)]
    pub display_type: Option<String>,
    /// Range bounds and default are polymorphic by `value_type`: floats for knobs
    /// (`valueType: 1`), booleans for switches (`valueType: 2`), etc. Kept raw; use the
    /// [`Param::min_f64`] / [`Param::default_bool`] accessors for typed reads.
    #[serde(default)]
    pub min: Option<Value>,
    #[serde(default)]
    pub max: Option<Value>,
    #[serde(default)]
    pub default: Option<Value>,
    /// Parameter index within the block. Candidate for the on-wire address — verify
    /// against captures before relying on it.
    #[serde(default)]
    pub assign: Option<i64>,
    /// Enum labels, sub-ranges, and other per-param metadata we have not typed yet.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Param {
    /// The default value as `f64`, if it is numeric (knob-style params).
    pub fn default_f64(&self) -> Option<f64> {
        self.default.as_ref().and_then(Value::as_f64)
    }

    /// The default value as `bool`, if it is a boolean (switch-style params).
    pub fn default_bool(&self) -> Option<bool> {
        self.default.as_ref().and_then(Value::as_bool)
    }

    /// The lower bound as `f64`, if numeric.
    pub fn min_f64(&self) -> Option<f64> {
        self.min.as_ref().and_then(Value::as_f64)
    }

    /// The upper bound as `f64`, if numeric.
    pub fn max_f64(&self) -> Option<f64> {
        self.max.as_ref().and_then(Value::as_f64)
    }
}
