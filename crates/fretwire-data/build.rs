use std::path::PathBuf;

// The Line 6 reference data is not shipped in the repo. Tests that validate against it are gated on
// `have_bundled_data`, which we set only when a copy is available in the user's runtime data dir —
// the `fretwire import-data` cache ($FRETWIRE_DATA_DIR, else $XDG_DATA_HOME/fretwire/data, else
// ~/.local/share/fretwire/data). A clean public checkout with no imported data skips those tests.
// Keep this in sync with `fretwire_core::data_dir()`.
fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FRETWIRE_DATA_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("fretwire").join("data")
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(have_bundled_data)");
    let sym = data_dir().join("Helix.sym");
    if sym.exists() {
        println!("cargo::rustc-cfg=have_bundled_data");
    }
    println!("cargo::rerun-if-env-changed=FRETWIRE_DATA_DIR");
    println!("cargo::rerun-if-env-changed=XDG_DATA_HOME");
    println!("cargo::rerun-if-changed={}", sym.display());
}
