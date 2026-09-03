//! The "is there a newer release?" check — a single request, opt-in, once a day.
//!
//! fretwire otherwise makes no network connection at all (USB and, under serve mode, a local
//! socket), so this is the one place the program reaches out, and it is built to stay small:
//!
//! - **One `HEAD`** to the GitHub *releases/latest* URL, redirects not followed. The `Location`
//!   header names the newest tag (`…/releases/tag/v0.5.0`). No API, no JSON, no rate limit, and
//!   nothing about the user goes with it — the User-Agent is the bare word `fretwire`, without
//!   even the version.
//! - **Opt-in.** Nothing is sent until the user has said yes (`enabled` is `None` until asked,
//!   and the front ends ask). `FRETWIRE_NO_UPDATE_CHECK=1` pins it off regardless of the file.
//! - **Once a day**, remembered in a small JSON file beside the data dir, so the GUI's startup
//!   check is usually just a file read.
//! - **Never in the way.** A short timeout, and the automatic check swallows every failure —
//!   offline is silence. Only an explicit check (`fretwire check-update`, the GUI's "Check now")
//!   reports an error.
//!
//! It only ever *reports*: nothing here downloads or installs anything. Each package channel has
//! its own updater (apt, dnf, an AUR helper) and a self-replacing binary would fight them; the
//! notice tells the user which of those to run, from how this binary was installed.
//!
//! A build from a checkout is usually *ahead* of the last tag, so "newer" is strict: equal or
//! behind is silence.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where the latest release lives. GitHub answers with a redirect to the tag's page.
pub const RELEASES_LATEST: &str = "https://github.com/john-baxter-dev/fretwire/releases/latest";

/// The version of this build — the workspace version, which every crate shares.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// How long a result is trusted before the automatic check asks again.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Ceiling on the whole request. The GUI runs the check off the UI thread, but a hung request
/// would still hold a `spawn_blocking` slot, and the CLI's user is waiting at a prompt.
const TIMEOUT: Duration = Duration::from_secs(6);

/// Environment variable that pins the check off, whatever the preference file says.
pub const ENV_DISABLE: &str = "FRETWIRE_NO_UPDATE_CHECK";

/// Environment variable overriding [`RELEASES_LATEST`] — for exercising the flow against a local
/// server; not a user-facing setting.
const ENV_URL: &str = "FRETWIRE_UPDATE_URL";

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("update check failed: {0}")]
    Http(String),
    /// The server answered with something other than the redirect GitHub uses for
    /// `releases/latest` — a 404 if the repository has no release yet, a 200 from a captive
    /// portal, and so on.
    #[error("update check: expected a redirect to the latest release, got HTTP {0}")]
    NotRedirected(u16),
    #[error("update check: cannot read a version out of the redirect target `{0}`")]
    BadLocation(String),
    #[error("update check: {0}")]
    Io(#[from] std::io::Error),
}

/// How this binary got onto the machine — which decides what "update" means for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Running out of an AppImage (`APPIMAGE` is set by its runtime).
    AppImage,
    /// Installed by the system package manager (`/usr/bin`, from the `.deb`, `.rpm` or the AUR).
    Package,
    /// `cargo install` (`~/.cargo/bin`).
    Cargo,
    /// Running from a build directory — a checkout.
    Source,
    Unknown,
}

impl InstallKind {
    /// Classify the running executable.
    pub fn detect() -> InstallKind {
        if std::env::var_os("APPIMAGE").is_some() {
            return InstallKind::AppImage;
        }
        match std::env::current_exe() {
            Ok(exe) => InstallKind::classify(&exe),
            Err(_) => InstallKind::Unknown,
        }
    }

    /// The path rule alone, so it can be tested without an executable to run.
    pub fn classify(exe: &Path) -> InstallKind {
        let s = exe.to_string_lossy();
        if s.starts_with("/usr/bin/") || s.starts_with("/usr/lib/") || s.starts_with("/opt/") {
            InstallKind::Package
        } else if s.contains("/.cargo/bin/") {
            InstallKind::Cargo
        } else if exe.components().any(|c| c.as_os_str() == "target") {
            InstallKind::Source
        } else {
            InstallKind::Unknown
        }
    }

    /// A stable slug for the wire / the CLI.
    pub fn as_str(&self) -> &'static str {
        match self {
            InstallKind::AppImage => "appimage",
            InstallKind::Package => "package",
            InstallKind::Cargo => "cargo",
            InstallKind::Source => "source",
            InstallKind::Unknown => "unknown",
        }
    }

    /// How the user sees it.
    pub fn label(&self) -> &'static str {
        match self {
            InstallKind::AppImage => "AppImage",
            InstallKind::Package => "system package",
            InstallKind::Cargo => "cargo install",
            InstallKind::Source => "built from a checkout",
            InstallKind::Unknown => "unknown install",
        }
    }

    /// What to do about a newer release, for this kind of install.
    pub fn instruction(&self) -> &'static str {
        match self {
            InstallKind::AppImage => {
                "Download the new AppImage from the release page and replace this one."
            }
            InstallKind::Package => {
                "Download the new .deb or .rpm from the release page and install it with apt or \
                 dnf (on Arch, update the AUR package)."
            }
            InstallKind::Cargo => "Run the same `cargo install` again to pick up the new version.",
            InstallKind::Source => "Pull the repository and rebuild.",
            InstallKind::Unknown => "See the release page.",
        }
    }
}

/// What the preference file holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prefs {
    /// `None` until the user has answered — the front ends ask while it is unset.
    pub enabled: Option<bool>,
    /// Unix seconds of the last completed probe.
    pub checked_at: Option<u64>,
    /// What that probe found, so the notice survives a relaunch within the day.
    pub latest: Option<String>,
}

/// `~/.local/share/fretwire/update-check.json` — beside the data dir (and `serve-token`), so
/// `$FRETWIRE_DATA_DIR` moves it too.
pub fn prefs_path() -> PathBuf {
    let data = crate::data_dir();
    data.parent()
        .map(Path::to_path_buf)
        .unwrap_or(data)
        .join("update-check.json")
}

/// Read the file; absent or unreadable means "never asked", never an error.
pub fn load_prefs(path: &Path) -> Prefs {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Prefs::default();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Prefs::default();
    };
    Prefs {
        enabled: v.get("enabled").and_then(|e| e.as_bool()),
        checked_at: v.get("checked_at").and_then(|c| c.as_u64()),
        latest: v.get("latest").and_then(|l| l.as_str()).map(str::to_string),
    }
}

pub fn save_prefs(path: &Path, prefs: &Prefs) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let v = serde_json::json!({
        "enabled": prefs.enabled,
        "checked_at": prefs.checked_at,
        "latest": prefs.latest,
    });
    std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(&v)?))
}

/// `FRETWIRE_NO_UPDATE_CHECK` is set to anything but empty or `0`.
pub fn env_disabled() -> bool {
    std::env::var_os(ENV_DISABLE).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Everything a front end needs to show: what we run, what is out there, and the preference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub current: String,
    /// The newest release known — from this probe or the cached one. `None` until a probe has
    /// succeeded.
    pub latest: Option<String>,
    /// `latest` is strictly newer than `current`.
    pub available: bool,
    /// The release page for `latest`, when it is newer.
    pub url: Option<String>,
    /// The user's answer; `None` = not asked yet. Always `Some(false)` when `locked`.
    pub enabled: Option<bool>,
    /// The environment pins the check off; the preference cannot turn it on.
    pub locked: bool,
    pub checked_at: Option<u64>,
    pub install: InstallKind,
}

impl Status {
    fn from_prefs(prefs: &Prefs) -> Status {
        let locked = env_disabled();
        let available = prefs
            .latest
            .as_deref()
            .is_some_and(|l| is_newer(l, CURRENT));
        Status {
            current: CURRENT.to_string(),
            latest: prefs.latest.clone(),
            available,
            url: available
                .then(|| prefs.latest.as_deref().map(release_url))
                .flatten(),
            enabled: if locked { Some(false) } else { prefs.enabled },
            locked,
            checked_at: prefs.checked_at,
            install: InstallKind::detect(),
        }
    }
}

/// The release page for a version.
pub fn release_url(version: &str) -> String {
    format!("https://github.com/john-baxter-dev/fretwire/releases/tag/v{version}")
}

/// The current state, from the file alone. No network.
pub fn status() -> Status {
    Status::from_prefs(&load_prefs(&prefs_path()))
}

/// Record the user's answer.
pub fn set_enabled(enabled: bool) -> Result<Status, UpdateError> {
    let path = prefs_path();
    let mut prefs = load_prefs(&path);
    prefs.enabled = Some(enabled);
    save_prefs(&path, &prefs)?;
    Ok(Status::from_prefs(&prefs))
}

/// Run the check if it is due, and return what is known either way.
///
/// `force` is "the user asked, right now": it probes regardless of the preference, the
/// environment and the daily interval, and reports a failure. Without it this is the automatic
/// check — it only reaches out when enabled, not locked, and at least [`CHECK_INTERVAL`] since
/// the last time; otherwise it answers from the file. Callers of the automatic form should treat
/// an error as silence.
pub fn check(force: bool) -> Result<Status, UpdateError> {
    let path = prefs_path();
    let mut prefs = load_prefs(&path);
    if !force {
        let due = prefs
            .checked_at
            .is_none_or(|t| now().saturating_sub(t) >= CHECK_INTERVAL.as_secs());
        if env_disabled() || prefs.enabled != Some(true) || !due {
            return Ok(Status::from_prefs(&prefs));
        }
    }
    let url = std::env::var(ENV_URL).unwrap_or_else(|_| RELEASES_LATEST.to_string());
    let latest = probe(&url)?;
    prefs.latest = Some(latest);
    prefs.checked_at = Some(now());
    // A failure to remember the answer is not a failure to have found it.
    if let Err(e) = save_prefs(&path, &prefs) {
        tracing::warn!(%e, path = %path.display(), "could not write the update-check file");
    }
    Ok(Status::from_prefs(&prefs))
}

/// The one request: `HEAD` the URL, don't follow the redirect, read the version off its target.
pub fn probe(url: &str) -> Result<String, UpdateError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .user_agent("fretwire")
        .build()
        .into();
    let resp = agent
        .head(url)
        .call()
        .map_err(|e| UpdateError::Http(e.to_string()))?;
    let code = resp.status().as_u16();
    if !(300..400).contains(&code) {
        return Err(UpdateError::NotRedirected(code));
    }
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    version_from_location(&location).ok_or(UpdateError::BadLocation(location))
}

/// `…/releases/tag/v0.5.0` → `0.5.0`. The `v` is optional; a query string or fragment is dropped.
pub fn version_from_location(location: &str) -> Option<String> {
    let last = location
        .split(['?', '#'])
        .next()?
        .trim_end_matches('/')
        .rsplit('/')
        .next()?;
    let v = last.strip_prefix('v').unwrap_or(last);
    parse_version(v).map(|_| v.to_string())
}

/// `MAJOR.MINOR.PATCH`, with anything after a `-` or `+` ignored. `None` if that is not what
/// the string is.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next()??;
    let patch = parts.next()??;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Strictly newer. Unparseable on either side is "not newer" — a notice should never rest on a
/// string we could not read.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("v1.2.3"), None);
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("0.4.1", "0.4.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.4.0", "0.4.0"), "equal is not newer");
        assert!(
            !is_newer("0.4.0", "0.5.0"),
            "a checkout ahead of the release stays quiet"
        );
        assert!(!is_newer("nonsense", "0.4.0"));
        assert!(!is_newer("0.5.0", "nonsense"));
    }

    #[test]
    fn location_yields_the_tag() {
        let base = "https://github.com/john-baxter-dev/fretwire/releases/tag/";
        assert_eq!(
            version_from_location(&format!("{base}v0.5.0")).as_deref(),
            Some("0.5.0")
        );
        assert_eq!(
            version_from_location(&format!("{base}0.5.0")).as_deref(),
            Some("0.5.0")
        );
        assert_eq!(
            version_from_location(&format!("{base}v0.5.0/")).as_deref(),
            Some("0.5.0")
        );
        assert_eq!(
            version_from_location(&format!("{base}v0.5.0?x=1#y")).as_deref(),
            Some("0.5.0")
        );
        assert_eq!(version_from_location("https://github.com/login"), None);
        assert_eq!(version_from_location(""), None);
    }

    #[test]
    fn install_kind_from_path() {
        use InstallKind::*;
        assert_eq!(classify("/usr/bin/fretwire-gui"), Package);
        assert_eq!(classify("/opt/fretwire/bin/fretwire"), Package);
        assert_eq!(classify("/home/me/.cargo/bin/fretwire"), Cargo);
        assert_eq!(
            classify("/home/me/src/fretwire/target/release/fretwire"),
            Source
        );
        assert_eq!(
            classify("/home/me/src/fretwire/target/debug/fretwire-gui"),
            Source
        );
        assert_eq!(classify("/usr/local/bin/fretwire"), Unknown);
        assert_eq!(
            classify("/tmp/.mount_fretwiXYZ/usr/bin/fretwire-gui"),
            Unknown
        );
        fn classify(p: &str) -> InstallKind {
            InstallKind::classify(Path::new(p))
        }
    }

    #[test]
    fn prefs_round_trip_and_absence() {
        let dir = std::env::temp_dir().join(format!("fretwire-update-{}", std::process::id()));
        let path = dir.join("nested").join("update-check.json");
        assert_eq!(load_prefs(&path), Prefs::default(), "no file = never asked");
        let prefs = Prefs {
            enabled: Some(true),
            checked_at: Some(1_700_000_000),
            latest: Some("0.5.0".into()),
        };
        save_prefs(&path, &prefs).unwrap();
        assert_eq!(load_prefs(&path), prefs);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(
            load_prefs(&path),
            Prefs::default(),
            "garbage reads as never asked"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn status_only_flags_a_strictly_newer_release() {
        let mut p = Prefs {
            latest: Some("0.0.1".into()),
            ..Prefs::default()
        };
        let s = Status::from_prefs(&p);
        assert!(!s.available && s.url.is_none());
        p.latest = Some("999.0.0".into());
        let s = Status::from_prefs(&p);
        assert!(s.available);
        assert_eq!(s.url.as_deref(), Some(release_url("999.0.0").as_str()));
        assert_eq!(s.current, CURRENT);
    }

    /// A local server standing in for GitHub: the probe must send a HEAD, not follow the
    /// redirect, and read the version off `Location`.
    fn serve_once(status_line: &'static str, location: Option<&'static str>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert!(
                request.starts_with("HEAD "),
                "expected a HEAD, got {request:?}"
            );
            let mut ua = None;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("user-agent:") {
                    ua = Some(v.trim().to_string());
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            assert_eq!(ua.as_deref(), Some("fretwire"), "the UA carries no version");
            let mut response = format!("HTTP/1.1 {status_line}\r\n");
            if let Some(l) = location {
                response.push_str(&format!("Location: {l}\r\n"));
            }
            response.push_str("Content-Length: 0\r\nConnection: close\r\n\r\n");
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{addr}/releases/latest")
    }

    #[test]
    fn probe_reads_the_redirect() {
        let url = serve_once(
            "302 Found",
            Some("https://github.com/john-baxter-dev/fretwire/releases/tag/v0.5.0"),
        );
        assert_eq!(probe(&url).unwrap(), "0.5.0");
    }

    #[test]
    fn probe_refuses_a_non_redirect() {
        let url = serve_once("200 OK", None);
        assert!(matches!(probe(&url), Err(UpdateError::NotRedirected(200))));
        let url = serve_once("404 Not Found", None);
        assert!(matches!(probe(&url), Err(UpdateError::NotRedirected(404))));
    }

    #[test]
    fn probe_refuses_a_redirect_elsewhere() {
        let url = serve_once("302 Found", Some("https://github.com/login"));
        assert!(matches!(probe(&url), Err(UpdateError::BadLocation(_))));
    }

    #[test]
    fn probe_reports_a_dead_host() {
        // A port nothing listens on: the bind-then-drop leaves it closed.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(matches!(
            probe(&format!("http://127.0.0.1:{port}/")),
            Err(UpdateError::Http(_))
        ));
    }
}
