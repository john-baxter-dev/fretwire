//! The half of the surface that needs no pedal: the model catalog and fretwire export files.

use fretwire_commands::dto::{PresetDto, PresetListItem};
use fretwire_core::backup::Backup;
use fretwire_core::editor::Catalog;
use fretwire_core::fretwire_protocol::DEVICES;
use fretwire_core::import::{self, DataFamily};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Which reference data a lookup reads. Each device family indexes its own symbol table, so a
/// POD Go's model 600 is not the HX family's model 600 — reading one against the other's catalog
/// names every model wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A device's, by the model code it stamps into its presets. Routed as `Session::connect`
    /// routes it, so a device without a known code reads the HX family's.
    Device(Option<&'static str>),
    /// No device in particular: the HX family's, or the POD Go's when that is all that is imported.
    Any,
}

/// The catalogs, one per device family, each loaded on first use and kept. Loading parses the
/// whole model table (a second or so), which is not worth paying at startup for a session that
/// may never need it. A failed load is not kept, so importing the data mid-session takes effect
/// on the next call rather than after a restart.
#[derive(Default)]
pub struct Offline {
    hx: OnceLock<Catalog>,
    pod_go: OnceLock<Catalog>,
}

impl Offline {
    pub fn catalog(&self, pick: Pick) -> Result<&Catalog, String> {
        match pick {
            Pick::Device(code) => self.catalog_for(code),
            Pick::Any => self.catalog_for(None).or_else(|hx| {
                self.catalog_for(pod_go_code()).map_err(|pg| {
                    if import::data_status().present {
                        // Something is imported and would not load: say what, not "import".
                        format!("{hx}\n{pg}")
                    } else {
                        format!(
                            "no Line 6 reference data in {} — run `fretwire import-data <path to \
                             an HX Edit or POD Go Edit installer, or its res folder>` once. {SHARED}",
                            fretwire_core::data_dir().display()
                        )
                    }
                })
            }),
        }
    }

    /// The catalog for the device stamping `model_code` (`None`: the HX family's).
    pub fn catalog_for(&self, model_code: Option<&str>) -> Result<&Catalog, String> {
        let family = model_code.map_or(import::HX, DataFamily::for_model_code);
        let slot = if family == import::POD_GO {
            &self.pod_go
        } else {
            &self.hx
        };
        // The core's error already names the family, the directory and the installer to import.
        cached(slot, || match model_code {
            Some(code) => Catalog::load_for_model(code),
            None => Catalog::load(),
        })
        .map_err(|e| format!("{e}. {SHARED}"))
    }

    /// The catalog a backed-up preset decodes against. The stream stamps the device it came off
    /// (key `7 -> 36`), which is what `fretwire show-preset` goes by too; failing that, the file's
    /// device tag; failing both, whichever family is imported.
    fn catalog_for_preset(&self, raw: &[u8], device: &str) -> Result<&Catalog, String> {
        use fretwire_core::fretwire_data::stream::PresetStream;
        if let Some(code) = PresetStream::parse(raw).ok().and_then(|s| s.device_model()) {
            return self.catalog_for(Some(&code));
        }
        match DEVICES.iter().find(|d| d.name == device) {
            Some(d) => self.catalog_for(d.model_code),
            None => self.catalog(Pick::Any),
        }
    }
}

const SHARED: &str = "The editor and this server share the imported data, and this server \
                      picks it up on the next call — no restart needed.";

/// The POD Go's model code, from the device table rather than spelled out here.
fn pod_go_code() -> Option<&'static str> {
    use fretwire_core::fretwire_protocol::{Device, PID_POD_GO};
    Device::by_pid(PID_POD_GO).and_then(|d| d.model_code)
}

/// `slot`'s value, loading it if it is empty and leaving it empty if the load fails.
/// `OnceLock::get_or_try_init` is unstable, hence the two steps: two first calls racing here
/// both load, the first to finish is kept and the other's copy dropped — a second spent twice,
/// never a wrong or torn value.
fn cached<T, E>(slot: &OnceLock<T>, load: impl FnOnce() -> Result<T, E>) -> Result<&T, E> {
    if let Some(v) = slot.get() {
        return Ok(v);
    }
    let loaded = load()?;
    Ok(slot.get_or_init(|| loaded))
}

/// A catalog tool's `device` argument, as the model code [`Offline::catalog_for`] routes on:
/// `"hx"`, or a device's name in any case and spacing (`"pod-go"`, `"POD Go"`, `"HX Stomp"`).
pub fn device_arg(device: &str) -> Result<Pick, String> {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let want = norm(device);
    if want == "hx" {
        return Ok(Pick::Device(None));
    }
    DEVICES
        .iter()
        .find(|d| norm(d.name) == want)
        .map(|d| Pick::Device(d.model_code))
        .ok_or_else(|| format!("unknown device {device:?} — use \"pod-go\" or \"hx\""))
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
    offline: &Offline,
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
    let preset = offline
        .catalog_for_preset(&entry.raw, &backup.device)?
        .load_preset(&entry.raw)
        .map_err(|e| format!("decoding preset {}: {e}", entry.name))?;
    let mut dto = PresetDto::from(&preset);
    // An offline decode carries no identity of its own; the file's entry is the identity.
    dto.name = Some(entry.name.clone());
    dto.index = Some(index);
    dto.bank = Some(bank);
    Ok(dto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fretwire_core::backup::BackupPreset;

    fn fixture(name: &str) -> Option<Vec<u8>> {
        std::fs::read(format!(
            "{}/../../captures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .ok()
    }

    fn one_preset(device: &str, raw: Vec<u8>) -> Backup {
        Backup {
            device: device.into(),
            presets: vec![BackupPreset {
                bank: 0,
                index: 0,
                name: "fixture".into(),
                raw,
            }],
            ..Default::default()
        }
    }

    fn symbols(p: &PresetDto) -> Vec<Option<String>> {
        p.blocks.iter().map(|b| b.symbolic_id.clone()).collect()
    }

    #[test]
    fn a_failed_load_is_not_kept_and_a_good_one_is() {
        let slot = OnceLock::new();
        assert_eq!(
            cached(&slot, || Err::<i32, _>("not imported")),
            Err("not imported")
        );
        assert!(slot.get().is_none(), "the failure stuck");
        assert_eq!(cached(&slot, || Ok::<_, &str>(1)), Ok(&1));
        assert_eq!(cached(&slot, || Ok::<_, &str>(2)), Ok(&1), "loaded twice");
    }

    #[test]
    fn the_device_argument_names_a_family_or_a_device() {
        assert_eq!(device_arg("pod-go"), Ok(Pick::Device(Some("P34"))));
        assert_eq!(device_arg("POD Go"), Ok(Pick::Device(Some("P34"))));
        assert_eq!(device_arg("hx"), Ok(Pick::Device(None)));
        assert_eq!(device_arg("HX Stomp"), Ok(Pick::Device(Some("P33"))));
        assert_eq!(device_arg("helix-floor"), Ok(Pick::Device(Some("P21"))));
        assert!(device_arg("pod").is_err());
    }

    // The rest need the reference data imported; each skips where its family is not.

    #[test]
    fn each_family_has_its_own_catalog() {
        let off = Offline::default();
        if let Ok(pg) = off.catalog_for(Some("P34")) {
            assert!(pg.pod_go());
            // Brit 2203 is index 600 on the POD Go, whose table is not the HX family's.
            assert_eq!(
                pg.model_params(600).expect("POD Go model 600").0,
                "Brit 2203"
            );
        }
        if let Ok(hx) = off.catalog_for(None) {
            assert!(!hx.pod_go());
            // Every HX code shares the one HX table, loaded once.
            assert!(std::ptr::eq(hx, off.catalog_for(Some("P33")).unwrap()));
        }
    }

    #[test]
    fn no_device_in_particular_reads_whichever_family_is_imported() {
        let off = Offline::default();
        let hx = off.catalog_for(None).is_ok();
        let pg = off.catalog_for(Some("P34")).is_ok();
        assert_eq!(off.catalog(Pick::Any).is_ok(), hx || pg);
        if hx {
            assert!(
                !off.catalog(Pick::Any).unwrap().pod_go(),
                "HX is the default"
            );
        }
    }

    /// The bug a fixed HX default would have: a POD Go preset read against the HX table names
    /// every block wrong. The stream says which device it came off, so the file needs no help —
    /// not even a device tag.
    #[test]
    fn a_pod_go_backup_decodes_against_pod_go_data() {
        let Some(raw) = fixture("pod-go/usdeluxe-factory01a.msgpack.bin") else {
            return;
        };
        let off = Offline::default();
        let Ok(pg) = off.catalog_for(Some("P34")) else {
            return;
        };
        let want = symbols(&PresetDto::from(&pg.load_preset(&raw).unwrap()));
        assert!(want.contains(&Some("HD2_WahFassel".into())), "{want:?}");
        for tag in ["POD Go", "unknown"] {
            let got = backup_preset(&off, &one_preset(tag, raw.clone()), 0, 0).unwrap();
            assert_eq!(symbols(&got), want, "tagged {tag:?}");
        }
        if let Ok(hx) = off.catalog_for(None)
            && let Ok(wrong) = hx.load_preset(&raw)
        {
            assert_ne!(symbols(&PresetDto::from(&wrong)), want);
        }
    }

    #[test]
    fn an_hx_backup_still_decodes_against_hx_data() {
        let Some(raw) = fixture("preset1_stream.msgpack.bin") else {
            return;
        };
        let off = Offline::default();
        let Ok(hx) = off.catalog_for(None) else {
            return;
        };
        let want = symbols(&PresetDto::from(&hx.load_preset(&raw).unwrap()));
        let got = backup_preset(&off, &one_preset("HX Stomp", raw), 0, 0).unwrap();
        assert_eq!(symbols(&got), want);
    }
}
