//! Typed parsers for the data files HX Edit ships in its `res/` folder.
//!
//! These describe the *semantic* layer of the device (models, parameters, presets,
//! display formatting) and require no protocol decoding — Line 6 ships them as JSON.
//! The wire protocol that drives a physical device lives in the `fretwire-protocol` / `fretwire-usb`
//! crates and is recovered separately via USB capture.

pub mod hxb;
pub mod ir;
pub mod modeldefs;
pub mod models;
pub mod preset;
pub mod stream;
pub mod symbols;
pub mod tone;

/// The MessagePack value type. Already part of this crate's public surface (`stream::map_get` and
/// friends take and return it), so re-export it rather than making every caller depend on the exact
/// `rmpv` version we do.
pub use rmpv;

pub use models::{Model, ModelFile, Param};
pub use preset::Preset;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("preset stream: {0}")]
    Stream(String),
}

pub type Result<T> = std::result::Result<T, Error>;
