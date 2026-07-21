fn main() {
    // The webview assets in ./dist are embedded into the binary by `generate_context!` at compile
    // time, but tauri-build only watches tauri.conf.json — NOT the dist dir. Without this, an
    // `npm run build` after the last cargo build silently ships a stale UI (cargo sees no Rust
    // change and skips the recompile that would re-embed the assets).
    println!("cargo:rerun-if-changed=dist");
    tauri_build::build();
}
