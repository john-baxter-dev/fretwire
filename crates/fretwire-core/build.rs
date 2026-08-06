use std::path::PathBuf;

// `have_bundled_data` gates tests that validate against the (unshipped) Line 6 reference data. It is
// set when a copy is available in the user's runtime data dir — the `fretwire import-data` cache
// ($FRETWIRE_DATA_DIR, else $XDG_DATA_HOME/fretwire/data, else ~/.local/share/fretwire/data). A
// clean checkout with no imported data simply skips those tests. Keep this in sync with
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

// The commit a binary was built from, for `FRETWIRE_BUILD_ID`. Testers send logs, not builds — and
// with only a version string a log from a source build is unidentifiable, which is how ~70 field
// logs came in with no way to tell which of them predated a given fix. `unknown` when the tree is
// not a git checkout (a release tarball, an AUR build), which is fine: those carry a real version.
//
// `--untracked-files=no` on purpose: a stray scratch file in the working tree is not a code change,
// and flagging it `-dirty` would cry wolf on every checkout that has one.
fn build_id() -> String {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    let Some(sha) = git(&["rev-parse", "--short=12", "HEAD"]) else {
        return "unknown".into();
    };
    let sha = String::from_utf8_lossy(&sha.stdout).trim().to_string();
    if sha.is_empty() {
        return "unknown".into();
    }
    match git(&["status", "--porcelain", "--untracked-files=no"]) {
        Some(o) if !o.stdout.is_empty() => format!("{sha}-dirty"),
        _ => sha,
    }
}

fn main() {
    println!("cargo::rustc-env=FRETWIRE_BUILD_ID={}", build_id());
    // Rebuild when the checked-out commit moves. Only emitted when `.git` is a real directory —
    // naming a path that does not exist makes cargo re-run this script on every single build, and
    // in a worktree or submodule `.git` is a file whose HEAD lives elsewhere.
    let git_dir = std::path::Path::new("../../.git");
    if git_dir.is_dir() {
        println!("cargo::rerun-if-changed=../../.git/HEAD");
        let head_ref = git_dir.join("refs/heads");
        if head_ref.is_dir() {
            println!("cargo::rerun-if-changed=../../.git/refs/heads");
        }
    }

    println!("cargo::rustc-check-cfg=cfg(have_bundled_data)");
    let sym = data_dir().join("Helix.sym");
    if sym.exists() {
        println!("cargo::rustc-cfg=have_bundled_data");
    }
    println!("cargo::rerun-if-env-changed=FRETWIRE_DATA_DIR");
    println!("cargo::rerun-if-env-changed=XDG_DATA_HOME");
    println!("cargo::rerun-if-changed={}", sym.display());
}
