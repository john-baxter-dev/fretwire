//! Editor session model: connect to a device, sync state, set parameters, load/save
//! presets, drive snapshots and the tuner.
//!
//! Status: the **offline editor model** ([`editor`]) is built — it decodes a preset stream into
//! typed, named, editable blocks by composing `fretwire-data` (preset + model + param-order resolution)
//! and `fretwire-protocol` (edit command building). The live USB session (sync over `fretwire-usb`) is still
//! to come (Phase 5).

pub mod backup;
pub mod editor;
pub mod import;
pub mod session;

pub use editor::{Catalog, EditorBlock, EditorParam, EditorPreset, ModelChoice};
pub use session::Session;

/// Re-export the building blocks so the CLI and GUI depend on one crate.
pub use fretwire_data;
pub use fretwire_protocol;
pub use fretwire_usb;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("preset data: {0}")]
    Data(#[from] fretwire_data::Error),
    #[error("protocol: {0}")]
    Protocol(#[from] fretwire_protocol::Error),
    #[error("usb: {0}")]
    Usb(#[from] fretwire_usb::Error),
    #[error("the pedal refused the {0}")]
    Rejected(String),
    #[error("backup file: {0}")]
    Backup(String),
    #[error("reference data: {0}")]
    MissingData(String),
    #[error("importing reference data: {0}")]
    Import(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The directory where the tool looks for the Line 6 reference data (`.models`, `Helix.sym`,
/// `HelixModelDefs.bin`, …) imported from a user's HX Edit install. Resolution order:
/// `$FRETWIRE_DATA_DIR` → `$XDG_DATA_HOME/fretwire/data` → `$HOME/.local/share/fretwire/data` → `./fretwire-data`.
///
/// [`editor::Catalog::load`] reads from here at runtime (preferred), falling back to the
/// build-time embedded copy only when the `bundled-data` feature is on and this dir is absent.
/// `fretwire import-data` writes here.
pub fn data_dir() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(d) = std::env::var_os("FRETWIRE_DATA_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("fretwire").join("data")
}
