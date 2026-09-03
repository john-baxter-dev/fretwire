//! The half of the surface that needs no pedal: the model catalog and fretwire export files.

use fretwire_commands::dto::PresetDto;
use fretwire_core::backup::Backup;
use fretwire_core::editor::Catalog;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The catalog, loaded on first use and kept. Loading parses the whole model table (a second or
/// so), which is not worth paying at startup for a session that may never need it.
#[derive(Default)]
pub struct Offline {
    catalog: OnceLock<Result<Catalog, String>>,
}

impl Offline {
    pub fn catalog(&self) -> Result<&Catalog, String> {
        self.catalog
            .get_or_init(|| {
                Catalog::load().map_err(|e| {
                    format!(
                        "the Line 6 reference data is not imported on this machine ({e}). Run \
                         `fretwire import-data <path to an HX Edit installer or its res folder>` \
                         once; the editor and this server share the result."
                    )
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// `~/` → `$HOME`, as the editor's own backup paths do.
pub fn expand_path(p: &str) -> PathBuf {
    let p = p.trim();
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(p)
}

pub fn read_backup(path: &str) -> Result<Backup, String> {
    let target = expand_path(path);
    let text = std::fs::read_to_string(&target)
        .map_err(|e| format!("reading {}: {e}", target.display()))?;
    Backup::from_json(&text).map_err(|e| format!("{}: {e}", target.display()))
}

/// One preset out of an export file, decoded through the catalog into the same DTO the live
/// path produces — so one summarizer serves both.
pub fn backup_preset(
    catalog: &Catalog,
    backup: &Backup,
    bank: i64,
    index: i64,
) -> Result<PresetDto, String> {
    let entry = backup.preset(bank, index).ok_or_else(|| {
        let have: Vec<String> = backup
            .presets
            .iter()
            .map(|p| format!("{}:{}", p.bank, p.index))
            .collect();
        format!(
            "the file has no preset at bank {bank} slot {index} (it has bank:slot {})",
            have.join(", ")
        )
    })?;
    let preset = catalog
        .load_preset(&entry.raw)
        .map_err(|e| format!("decoding preset {}: {e}", entry.name))?;
    let mut dto = PresetDto::from(&preset);
    // An offline decode carries no identity of its own; the file's entry is the identity.
    dto.name = Some(entry.name.clone());
    dto.index = Some(index);
    dto.bank = Some(bank);
    Ok(dto)
}
