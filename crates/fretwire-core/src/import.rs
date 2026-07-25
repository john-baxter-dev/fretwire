//! Importing Line 6's reference data from the user's **own** HX Edit installation.
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
pub const REQUIRED: &str = "Helix.sym";

/// Files that must be present for model names and parameter ordering to resolve.
const ESSENTIAL: [&str; 2] = [REQUIRED, "HelixModelDefs.bin"];

/// What [`import_from`] copied.
#[derive(Debug, Clone)]
pub struct ImportSummary {
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
    /// Whether [`REQUIRED`] is present — i.e. whether `Catalog::load()` will succeed.
    pub present: bool,
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
    let files = std::fs::read_dir(&dir)
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
        .unwrap_or(0);
    DataStatus {
        present: dir.join(REQUIRED).is_file(),
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

    let mut copied = 0usize;
    for (name, src) in &by_name {
        std::fs::copy(src, dest.join(name))
            .map_err(|e| err(format!("copying {name} → {}: {e}", dest.display())))?;
        copied += 1;
    }
    cleanup();

    let missing = ESSENTIAL
        .iter()
        .filter(|n| !dest.join(n).exists())
        .map(|n| n.to_string())
        .collect();
    Ok(ImportSummary {
        copied,
        dest,
        missing,
    })
}

/// Whether `name` is a reference-data file we import. `.hlx` is restricted to the default/empty
/// templates (not the hundreds of factory presets an installer may also carry).
pub fn is_reference_file(name: &str) -> bool {
    name.ends_with(".models")
        || matches!(
            name,
            "Helix.sym"
                | "HelixModelDefs.bin"
                | "HelixControls.json"
                | "HX_ModelCatalog.json"
                | "HX_ModelCatalog.bin"
                | "default_preset.hlx"
                | "default_preset_hxs.hlx"
                | "default_preset_hfx.hlx"
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
    fn missing_source_is_an_error() {
        assert!(import_from(Path::new("/nonexistent/fretwire/source")).is_err());
    }
}
