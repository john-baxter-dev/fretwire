//! Importing Line 6's reference data from the user's **own** HX Edit / POD Go Edit installation.
//!
//! We ship none of this data; it goes Line6 → user → tool (the emulator "bring your own BIOS"
//! pattern). This module owns the mechanics so both front ends can offer it: `fretwire import-data`
//! on the CLI, and the GUI's first-run screen.
//!
//! The source may be either:
//!   - a **directory** — already-extracted files (e.g. the `res/` folder of an HX Edit install, or
//!     an installer unpacked by any means). Scanned in place; needs no `7z`.
//!   - an **installer file** — NSIS `.exe`, `.msi`, macOS `.pkg`/`.dmg`, unpacked with `7z` into a
//!     temp dir first.
//!
//! In both cases the wanted files are located by name anywhere under the source.

use std::path::{Path, PathBuf};

/// The one file the catalog can't work without — its presence is what marks a data dir as usable
/// (see [`crate::editor::Catalog::load`], which gates on the same file).
///
/// This is the **HX family's** spelling. A POD Go install names the same file `PodGo.sym`; use
/// [`DataFamily::detect`] rather than this constant when either family may be in play.
pub const REQUIRED: &str = "Helix.sym";

/// One device family's reference-data filenames.
///
/// Line 6 ships the same four formats under different names per editor, so the loader and the
/// importer both resolve through this table instead of hard-coding HX Edit's spelling. The
/// `.models` files are named identically in both — which is exactly why a non-HX family imports
/// into its own [`subdir`](DataFamily::subdir) rather than on top of an existing HX install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataFamily {
    /// Human name, for messages.
    pub label: &'static str,
    /// Subdirectory of [`crate::data_dir`] this family's files live in. Empty for the HX family,
    /// which keeps the flat layout every existing install already has.
    pub subdir: &'static str,
    /// Per-model parameter ordering — the file a block's model reference indexes.
    pub symbols: &'static str,
    /// The model table carrying display names and categories.
    pub model_defs: &'static str,
    /// Parameter ranges and control metadata.
    pub controls: &'static str,
    /// Category catalog.
    pub catalog_json: &'static str,
}

/// HX Edit's names, serving the HX Stomp / Helix / HX Effects family.
pub const HX: DataFamily = DataFamily {
    label: "HX Edit",
    subdir: "",
    symbols: REQUIRED,
    model_defs: "HelixModelDefs.bin",
    controls: "HelixControls.json",
    catalog_json: "HX_ModelCatalog.json",
};

/// POD Go Edit's names. Same formats, different spellings — and a symbol table of its own, which
/// is the whole reason the POD Go needs a separate import. [2026-08-25, issue #15]
pub const POD_GO: DataFamily = DataFamily {
    label: "POD Go Edit",
    subdir: "pod-go",
    symbols: "PodGo.sym",
    model_defs: "PodGoModelDefs.bin",
    controls: "PGControls.json",
    catalog_json: "PGModelCatalog.json",
};

/// Every family we can import, most specific first — [`DataFamily::detect`] scans in this order,
/// so the HX family (whose subdir is the data dir itself) is tried last.
pub const FAMILIES: &[DataFamily] = &[POD_GO, HX];

impl DataFamily {
    /// The family whose reference data sits in `dir`, by which symbol table is present.
    pub fn detect(dir: &Path) -> Option<DataFamily> {
        FAMILIES
            .iter()
            .copied()
            .find(|f| dir.join(f.symbols).is_file())
    }

    /// The family serving a device's model code (preset key `7 -> 36`). Unknown codes get the HX
    /// family, which is the right default: it is what every device but the POD Go uses.
    pub fn for_model_code(code: &str) -> DataFamily {
        if code == "P34" { POD_GO } else { HX }
    }

    /// Where this family's files live under `data_dir`.
    pub fn dir_under(&self, data_dir: &Path) -> PathBuf {
        if self.subdir.is_empty() {
            data_dir.to_path_buf()
        } else {
            data_dir.join(self.subdir)
        }
    }

    /// Files that must be present for model names and parameter ordering to resolve.
    fn essential(&self) -> [&'static str; 2] {
        [self.symbols, self.model_defs]
    }

    /// Whether `name` is one of this family's four named files.
    fn owns(&self, name: &str) -> bool {
        name == self.symbols
            || name == self.model_defs
            || name == self.controls
            || name == self.catalog_json
    }
}

/// What [`import_from`] copied.
#[derive(Debug, Clone)]
pub struct ImportSummary {
    /// Which vendor's data this was — `"HX Edit"` or `"POD Go Edit"`. The destination directory
    /// implies it, but only if you know the layout; say it outright.
    pub family: &'static str,
    /// Number of reference files copied into [`ImportSummary::dest`].
    pub copied: usize,
    /// Where they landed — [`crate::data_dir`].
    pub dest: PathBuf,
    /// Essential files that were *not* found in the source. Import still succeeds (the tool edits
    /// by raw parameter index without them), but names and ranges will be missing.
    pub missing: Vec<String>,
}

/// Whether the local data dir holds usable reference data, and what's in it.
#[derive(Debug, Clone)]
pub struct DataStatus {
    /// Whether **any** family's data is imported — i.e. whether the first-run screen still has a
    /// job to do. Not the same as "`Catalog::load()` will succeed": that one is specifically the HX
    /// family, so a POD Go owner who has imported only POD Go Edit is `present` and would still
    /// fail an HX load. Use [`DataStatus::families`] to ask about a particular device.
    pub present: bool,
    /// The families actually imported, by label — e.g. `["HX Edit", "POD Go Edit"]`. Empty when
    /// nothing has been imported. This is the field that answers "what do I have?" once more than
    /// one device family is in play; [`DataStatus::files`] is a total across all of them.
    pub families: Vec<&'static str>,
    /// The directory consulted ([`crate::data_dir`]).
    pub dir: PathBuf,
    /// How many reference files are cached there.
    pub files: usize,
}

/// Inspect the local data dir without loading the catalog. Cheap enough to call on GUI startup.
pub fn data_status() -> DataStatus {
    data_status_in(crate::data_dir())
}

/// [`data_status`] against an explicit directory.
pub fn data_status_in(dir: PathBuf) -> DataStatus {
    let count = |d: &Path| {
        std::fs::read_dir(d)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| {
                        e.file_name()
                            .to_str()
                            .map(is_reference_file)
                            .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0)
    };
    // Each family keeps its own directory, so a status is the sum over all of them.
    let files: usize = FAMILIES.iter().map(|f| count(&f.dir_under(&dir))).sum();
    let present = FAMILIES
        .iter()
        .any(|f| f.dir_under(&dir).join(f.symbols).is_file());
    DataStatus {
        present,
        families: FAMILIES
            .iter()
            .filter(|f| f.dir_under(&dir).join(f.symbols).is_file())
            .map(|f| f.label)
            .collect(),
        dir,
        files,
    }
}

/// Import reference data from `source` (an installer file or an extracted directory) into
/// [`crate::data_dir`], and report what landed.
pub fn import_from(source: &Path) -> crate::Result<ImportSummary> {
    import_into(source, crate::data_dir())
}

/// [`import_from`] into an explicit destination directory.
pub fn import_into(source: &Path, dest: PathBuf) -> crate::Result<ImportSummary> {
    if !source.exists() {
        return Err(err(format!("not found: {}", source.display())));
    }
    std::fs::create_dir_all(&dest).map_err(|e| err(format!("{}: {e}", dest.display())))?;

    // A directory source is scanned in place (no 7z); a file is unpacked into a temp dir we clean up.
    let tmp = if source.is_dir() {
        None
    } else {
        let t = std::env::temp_dir().join(format!("fretwire-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).map_err(|e| err(format!("{}: {e}", t.display())))?;
        if let Err(e) = unpack_with_7z(source, &t) {
            let _ = std::fs::remove_dir_all(&t);
            return Err(e);
        }
        Some(t)
    };
    let search_root = tmp.as_deref().unwrap_or(source);
    let cleanup = || {
        if let Some(t) = &tmp {
            let _ = std::fs::remove_dir_all(t);
        }
    };

    // Collect the wanted files anywhere under the source, de-duped by name (preferring the copy
    // under a `res` directory if the same name appears twice).
    let mut found: Vec<PathBuf> = Vec::new();
    collect_wanted(search_root, &mut found);
    let mut by_name: std::collections::BTreeMap<String, PathBuf> =
        std::collections::BTreeMap::new();
    for p in found {
        let name = match p.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let in_res = p.to_string_lossy().contains("res");
        let keep = match by_name.get(&name) {
            Some(prev) => in_res || !prev.to_string_lossy().contains("res"),
            None => true,
        };
        if keep {
            by_name.insert(name, p);
        }
    }
    if by_name.is_empty() {
        cleanup();
        return Err(err(if source.is_dir() {
            format!(
                "no reference data (Helix.sym, *.models, …) found under {} — point at an HX Edit \
                 install's `res/` folder or an unpacked installer",
                source.display()
            )
        } else {
            format!(
                "no reference data found in {} — is this an HX Edit installer?",
                source.display()
            )
        }));
    }

    // Which editor's data this is, by the symbol table it carries. Everything but the HX family
    // lands in its own subdirectory: the `.models` files are named identically across families, so
    // a flat copy would quietly overwrite an existing HX install with POD Go data.
    let family = FAMILIES
        .iter()
        .copied()
        .find(|f| by_name.contains_key(f.symbols))
        .unwrap_or(HX);
    let dest = family.dir_under(&dest);
    std::fs::create_dir_all(&dest).map_err(|e| err(format!("{}: {e}", dest.display())))?;

    let mut copied = 0usize;
    for (name, src) in &by_name {
        std::fs::copy(src, dest.join(name))
            .map_err(|e| err(format!("copying {name} → {}: {e}", dest.display())))?;
        copied += 1;
    }
    cleanup();

    let missing = family
        .essential()
        .iter()
        .filter(|n| !dest.join(n).exists())
        .map(|n| n.to_string())
        .collect();
    Ok(ImportSummary {
        family: family.label,
        copied,
        dest,
        missing,
    })
}

/// Whether `name` is a reference-data file we import. `.hlx` is restricted to the default/empty
/// templates (not the hundreds of factory presets an installer may also carry).
pub fn is_reference_file(name: &str) -> bool {
    name.ends_with(".models")
        || FAMILIES.iter().any(|f| f.owns(name))
        || matches!(
            name,
            "HX_ModelCatalog.bin"
                | "PGModelCatalog.bin"
                | "default_preset.hlx"
                | "default_preset_hxs.hlx"
                | "default_preset_hfx.hlx"
                | "default_preset_p34.hlx"
                | "empty_preset.hlx"
        )
}

/// Recursively collect reference-data files (by name) under `dir`.
fn collect_wanted(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_wanted(&path, out);
        } else if let Some(name) = path.file_name().and_then(|s| s.to_str())
            && is_reference_file(name)
        {
            out.push(path);
        }
    }
}

/// Unpack an installer into `tmp` using `7z`/`7za`. 7z reads NSIS `.exe`, `.msi`, and macOS
/// `.pkg`/`.dmg`. Exit code 1 (warnings) is tolerated; the wanted-files check is the real gate.
fn unpack_with_7z(installer: &Path, tmp: &Path) -> crate::Result<()> {
    use std::process::{Command, Stdio};
    for bin in ["7z", "7za"] {
        let result = Command::new(bin)
            .arg("x")
            .arg("-y")
            .arg(format!("-o{}", tmp.display()))
            .arg(installer)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match result {
            Ok(st) if matches!(st.code(), Some(0) | Some(1)) => return Ok(()),
            Ok(st) => {
                return Err(err(format!(
                    "{bin} failed to extract the installer (exit {st})"
                )));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(err(format!("running {bin}: {e}"))),
        }
    }
    Err(err(
        "need `7z` on PATH to unpack an installer (Arch: p7zip · Debian/Ubuntu: p7zip-full · \
         Fedora: p7zip · macOS: brew install p7zip) — or extract it yourself (or copy the `res/` \
         folder from an existing HX Edit install) and import that directory instead"
            .to_string(),
    ))
}

fn err(msg: String) -> crate::Error {
    crate::Error::Import(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_files_are_recognised_by_name() {
        assert!(is_reference_file("Helix.sym"));
        assert!(is_reference_file("amp.models"));
        assert!(is_reference_file("empty_preset.hlx"));
        // Factory presets ride along in an installer; we don't want the hundreds of them.
        assert!(!is_reference_file("US Double Nrm.hlx"));
        assert!(!is_reference_file("readme.txt"));
    }

    #[test]
    fn import_copies_reference_files_and_skips_the_rest() {
        let tmp = std::env::temp_dir().join(format!("fretwire-import-test-{}", std::process::id()));
        let src = tmp.join("res/nested");
        let dest = tmp.join("data");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("Helix.sym"), b"[]").unwrap();
        std::fs::write(src.join("amp.models"), b"{}").unwrap();
        std::fs::write(src.join("notes.txt"), b"nope").unwrap();

        let summary = import_into(&tmp.join("res"), dest.clone()).unwrap();
        assert_eq!(summary.copied, 2);
        assert!(dest.join("Helix.sym").is_file());
        assert!(!dest.join("notes.txt").exists());
        // HelixModelDefs.bin wasn't in the source — reported, but not fatal.
        assert_eq!(summary.missing, vec!["HelixModelDefs.bin".to_string()]);

        let status = data_status_in(dest);
        assert!(status.present);
        assert_eq!(status.files, 2);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pod_go_data_lands_in_its_own_subdir() {
        // The `.models` filenames are shared with HX Edit, so importing POD Go data on top of an
        // HX install would silently replace it. It gets its own directory instead.
        let tmp = std::env::temp_dir().join(format!("fretwire-podgo-test-{}", std::process::id()));
        let src = tmp.join("res");
        let dest = tmp.join("data");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        // An existing HX install we must not disturb.
        std::fs::write(dest.join("amp.models"), b"hx").unwrap();
        std::fs::write(src.join("PodGo.sym"), b"[]").unwrap();
        std::fs::write(src.join("PodGoModelDefs.bin"), b"\x90").unwrap();
        std::fs::write(src.join("amp.models"), b"podgo").unwrap();

        let summary = import_into(&src, dest.clone()).unwrap();
        assert_eq!(summary.dest, dest.join("pod-go"));
        assert_eq!(summary.copied, 3);
        assert!(summary.missing.is_empty());
        assert_eq!(std::fs::read(dest.join("amp.models")).unwrap(), b"hx");
        assert_eq!(
            std::fs::read(dest.join("pod-go/amp.models")).unwrap(),
            b"podgo"
        );

        // A status over the data dir sees both families.
        let status = data_status_in(dest);
        assert!(status.present);
        assert_eq!(status.files, 4);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn families_are_told_apart_by_their_symbol_table() {
        assert_eq!(DataFamily::for_model_code("P34"), POD_GO);
        assert_eq!(DataFamily::for_model_code("P33"), HX);
        // An unknown code is not a POD Go, and the HX family is the right default for the rest.
        assert_eq!(DataFamily::for_model_code("P99"), HX);
    }

    #[test]
    fn missing_source_is_an_error() {
        assert!(import_from(Path::new("/nonexistent/fretwire/source")).is_err());
    }
}
