use std::path::PathBuf;

// `have_bundled_data` gates the tests that render a real preset through the (unshipped) Line 6
// reference data — the same cfg fretwire-core and fretwire-data set, from the same directory.
// A clean checkout with no imported data simply skips them. Keep in sync with
// `fretwire_core::data_dir()`.
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
    println!("cargo::rerun-if-changed={}", sym.display());
    println!("cargo::rerun-if-env-changed=FRETWIRE_DATA_DIR");
    if sym.exists() {
        println!("cargo::rustc-cfg=have_bundled_data");
    }
}
