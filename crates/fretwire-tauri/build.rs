use std::path::Path;
use std::time::SystemTime;

fn main() {
    // ./dist is a build artifact and isn't in git, so a fresh clone hits `generate_context!`'s
    // "frontendDist ... doesn't exist" proc-macro panic. Fail earlier with the command to run.
    if !Path::new("dist/index.html").exists() {
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

    // The other direction, which nothing catches on its own: edit `ui/src`, run `cargo build`, and
    // you get a binary serving the *previous* frontend. Nothing rebuilds dist — there is no
    // `beforeBuildCommand`, and cargo has no idea `ui/src` exists. A contributor tested a UI fix
    // this way, saw no change, and reported the fix as broken (issue #13). A warning is the whole
    // remedy: this can't run npm itself (a build script must not, and a release build has already
    // run it), and it must not fail the build — `ui/` may legitimately be absent in a packaging
    // tree.
    println!("cargo:rerun-if-changed=ui/src");
    if let Some(stale) = newer_than_dist() {
        println!(
            "cargo:warning=the embedded frontend is older than {stale} — \
             run `npm run build` in crates/fretwire-tauri/ui, or this binary serves the old UI"
        );
    }

    // This call is what compiles `capabilities/default.json` into the binary. It resolves the
    // capability files against every plugin's permission manifest and writes the result into
    // OUT_DIR, where `generate_context!` picks it up; without it the webview's ACL is *empty*, and
    // every plugin call from the frontend — `listen`, the dialog — is refused with "not allowed.
    // Plugin not found", while app commands (`invoke`) keep working because they have no ACL of
    // their own. That is not a build error and not a log line: the IPC rejection lands only in
    // the webview's console. The 2026-08-25 rewrite of this script dropped the call by accident,
    // and live-follow, which is the `device-pushes` listener, silently stopped in every build
    // from then to v0.5.0 (issue #15, found by the POD Go owner from the JS console). It went
    // unnoticed here because cargo reuses OUT_DIR across builds with the same package metadata,
    // so a dev tree kept serving the ACL an earlier build had written — a version bump was what
    // emptied it, which is why the owner's bisect named the 0.4.0 bump. The test
    // `capabilities_are_compiled_in` in main.rs holds the door.
    tauri_build::build();
}

/// A path under `ui/src` modified after `dist/index.html` was written, if any.
fn newer_than_dist() -> Option<String> {
    let built = modified("dist/index.html")?;
    let mut stack = vec![Path::new("ui/src").to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if modified(&path).is_some_and(|m| m > built) {
                return Some(path.display().to_string());
            }
        }
    }
    None
}

fn modified<P: AsRef<Path>>(p: P) -> Option<SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}
