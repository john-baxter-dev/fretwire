use std::path::Path;

fn main() {
    // The frontend is shared with fretwire-tauri and lives in its dist/, which is a build
    // artifact and isn't in git — so a fresh clone would hit rust-embed's own missing-folder
    // error. Fail earlier, with the command to run (same guard as fretwire-tauri/build.rs).
    if !Path::new("../fretwire-tauri/dist/index.html").exists() {
        panic!(
            "the frontend hasn't been built — run:\n  \
             cd crates/fretwire-tauri/ui && npm install && npm run build"
        );
    }

    // Release builds embed dist/ at compile time; without this, an `npm run build` after the
    // last cargo build silently ships a stale UI (see fretwire-tauri/build.rs for the history).
    println!("cargo:rerun-if-changed=../fretwire-tauri/dist");
}
