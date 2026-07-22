fn main() {
    // ./dist is a build artifact and isn't in git, so a fresh clone hits `generate_context!`'s
    // "frontendDist ... doesn't exist" proc-macro panic. Fail earlier with the command to run.
    if !std::path::Path::new("dist/index.html").exists() {
        panic!(
            "the frontend hasn't been built — run:\n  \
             cd crates/fretwire-tauri/ui && npm install && npm run build"
        );
    }

    // The webview assets in ./dist are embedded into the binary by `generate_context!` at compile
    // time, but tauri-build only watches tauri.conf.json — NOT the dist dir. Without this, an
    // `npm run build` after the last cargo build silently ships a stale UI (cargo sees no Rust
    // change and skips the recompile that would re-embed the assets).
    println!("cargo:rerun-if-changed=dist");
    tauri_build::build();
}
