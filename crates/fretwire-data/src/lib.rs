//! Typed parsers for the data files HX Edit ships in its `res/` folder.
//!
//! These describe the *semantic* layer of the device (models, parameters, presets,
//! display formatting) and require no protocol decoding — Line 6 ships them as JSON.
//! The wire protocol that drives a physical device lives in the `fretwire-protocol` / `fretwire-usb`
//! crates and is recovered separately via USB capture.

pub mod hxb;
pub mod modeldefs;
pub mod models;
pub mod preset;
pub mod stream;
pub mod symbols;

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
