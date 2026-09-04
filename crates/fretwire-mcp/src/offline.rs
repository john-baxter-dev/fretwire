//! The half of the surface that needs no pedal: the model catalog and fretwire export files.

use fretwire_commands::dto::{PresetDto, PresetListItem};
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
    let bytes = std::fs::read(&target).map_err(|e| format!("reading {}: {e}", target.display()))?;
    if is_hxb(&bytes) {
        // HX Edit's own container. Its presets are `tone` JSON, not wire streams, and turning one
        // into a stream needs a donor preset from the same device (`fretwire hxb-convert`), so
        // the file is not read here; `backup_list` does list its names.
        return Err(format!(
            "{} is an HX Edit / POD Go Edit device backup (.hxb/.pgb), not a fretwire export. \
             backup_list reads its preset names; to describe or diff its presets, convert it first: \
             `fretwire hxb-convert <file> --donor <a fretwire export or stream from the same device> \
             <out.json>`.",
            target.display()
        ));
    }
    let text =
        String::from_utf8(bytes).map_err(|e| format!("{}: not text: {e}", target.display()))?;
    Backup::from_json(&text).map_err(|e| format!("{}: {e}", target.display()))
}

/// Whether `bytes` are an HX Edit / POD Go Edit backup container (`AF6L` magic).
pub fn is_hxb(bytes: &[u8]) -> bool {
    bytes.starts_with(b"AF6L")
}

/// The presets an HX Edit / POD Go Edit backup lists — names and slots per setlist — plus the
/// device it names. No conversion, no catalog: the container carries them in the clear.
pub fn hxb_list(path: &str) -> Result<Option<(String, Vec<PresetListItem>)>, String> {
    use fretwire_core::fretwire_data::hxb::Hxb;
    let target = expand_path(path);
    let bytes = std::fs::read(&target).map_err(|e| format!("reading {}: {e}", target.display()))?;
    if !is_hxb(&bytes) {
        return Ok(None);
    }
    let hxb = Hxb::parse(&bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    let device = fretwire_core::fretwire_usb::DEVICES
        .iter()
        .find(|d| d.preset_device_id == Some(hxb.device_id))
        .map(|d| d.name.to_string())
        .unwrap_or_else(|| format!("unknown device {:#010x}", hxb.device_id));
    let mut items = Vec::new();
    for setlist in hxb.setlists() {
        for p in setlist.presets.iter().flatten() {
            items.push(PresetListItem {
                label: None,
                index: p.index as i64,
                bank: setlist.bank as i64,
                setlist: Some(setlist.name.clone()),
                name: p.name.clone(),
            });
        }
    }
    Ok(Some((device, items)))
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
