//! The bearer token that gates a non-loopback bind (`docs/serve-mode.md` §4).
//!
//! Loopback needs none: only local processes reach the port. Anywhere wider, every invoke and
//! the event socket must present the token, which the daemon generates once, keeps in a
//! mode-0600 file, and prints at startup inside the link to open (`#token=…` — a fragment, so it
//! never reaches server logs or a `Referer`). The link is the credential.

use std::io::{Read, Write};
use std::path::Path;

/// Bytes of entropy in a generated token; it is printed as hex, so twice this many characters.
const TOKEN_BYTES: usize = 32;

/// Read the token from `path`, or generate one and write it there. Returns the token and whether
/// it was freshly created.
pub fn load_or_create(path: &Path) -> std::io::Result<(String, bool)> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let token = text.trim().to_string();
            if !token.is_empty() {
                return Ok((token, false));
            }
            // An empty file is treated as absent — regenerated, not accepted.
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let token = generate()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    writeln!(f, "{token}")?;
    Ok((token, true))
}

/// 32 bytes from the kernel, as hex. `/dev/urandom` directly: this is a Linux tool, and a crate
/// for one read is not worth the dependency.
fn generate() -> std::io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Constant-time comparison, so a wrong guess doesn't time differently by how much of it was
/// right. A length mismatch returns early — the length of a generated token is public anyway.
pub fn matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_is_exact() {
        assert!(matches("abc", "abc"));
        assert!(!matches("abc", "abd"));
        assert!(!matches("abc", "ab"));
        assert!(!matches("", "a"));
        assert!(matches("", ""));
    }

    #[test]
    fn the_file_round_trips_and_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("fretwire-token-{}", std::process::id()));
        let path = dir.join("nested").join("serve-token");
        let (first, created) = load_or_create(&path).unwrap();
        assert!(created);
        assert_eq!(first.len(), TOKEN_BYTES * 2, "hex of {TOKEN_BYTES} bytes");
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token file is owner-only");
        let (again, created) = load_or_create(&path).unwrap();
        assert!(!created);
        assert_eq!(again, first, "the stored token is reused");
        std::fs::write(&path, "\n").unwrap();
        let (fresh, created) = load_or_create(&path).unwrap();
        assert!(created, "an empty file is regenerated");
        assert_ne!(fresh, first);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
