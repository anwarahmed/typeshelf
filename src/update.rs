//! Self-update: at startup, compare this version with the latest GitHub release and,
//! if that is newer, download its binary over this one and restart.
//!
//! Only copies the user installed themselves update this way. A build run from a source
//! checkout, or a copy owned by a package manager (Homebrew, pacman), is left alone.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::log;
use crate::store::{Settings, checkout_root};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit this binary was built from, for `--version` and the log; empty if
/// unknown, `-dirty` if the tree had local changes.
pub const COMMIT: &str = env!("TYPESHELF_COMMIT");

const LATEST_URL: &str = "https://api.github.com/repos/anwarahmed/typeshelf/releases/latest";
const DOWNLOAD_URL: &str = "https://github.com/anwarahmed/typeshelf/releases/download";
/// Set on the restarted process so a failed or raced update can't loop.
const SKIP_ENV: &str = "TYPESHELF_NO_UPDATE";
/// Starting the app must not hang on a bad connection.
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

type Version = (u32, u32, u32);

/// Parses `1.2.3` or `v1.2.3`.
fn parse_version(s: &str) -> Option<Version> {
    let mut parts = s.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    match (parts.next()??, parts.next()??, parts.next()??, parts.next()) {
        (a, b, c, None) => Some((a, b, c)),
        _ => None,
    }
}

/// The release asset built for this platform, if there is one.
fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("typeshelf-x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("typeshelf-aarch64-unknown-linux-musl"),
        ("macos", "aarch64") => Some("typeshelf-aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("typeshelf-x86_64-apple-darwin"),
        _ => None,
    }
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".typeshelf-write-test-{}", std::process::id()));
    let ok = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(probe);
    ok
}

/// Why this copy doesn't update itself, if it doesn't.
fn skip_reason(settings: &Settings, exe: &Path) -> Option<&'static str> {
    if std::env::var_os(SKIP_ENV).is_some_and(|v| !v.is_empty()) {
        Some("TYPESHELF_NO_UPDATE is set")
    } else if !settings.auto_update {
        Some("turned off in settings")
    } else if checkout_root().is_some() {
        Some("running from a source checkout; use git pull and cargo build")
    } else if exe.components().any(|c| c.as_os_str() == "Cellar") {
        Some("installed with Homebrew; use brew upgrade typeshelf")
    } else if asset_name().is_none() {
        Some("no prebuilt binary for this platform; rebuild from source to update")
    } else if !exe.parent().is_some_and(writable) {
        Some("its directory is not writable, so a package manager probably owns it; update it the way you installed it")
    } else {
        None
    }
}

fn get(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(timeout)).build().into();
    let mut res = agent.get(url).header("User-Agent", "typeshelf").call().map_err(|e| e.to_string())?;
    res.body_mut().with_config().limit(64 << 20).read_to_vec().map_err(|e| e.to_string())
}

/// The newest released version and its tag.
fn latest_release() -> Result<(Version, String), String> {
    let body = get(LATEST_URL, CHECK_TIMEOUT)?;
    let json: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let tag = json["tag_name"].as_str().ok_or("unexpected reply from GitHub")?;
    Ok((parse_version(tag).ok_or_else(|| format!("unrecognized release tag {tag:?}"))?, tag.to_string()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Finds a file's checksum in `sha256sum` output (`<hex>  <name>` per line).
fn expected_sum<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|l| {
        let (sum, file) = l.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then_some(sum)
    })
}

/// Downloads the release binary for this platform, checks it against the release's
/// checksums, and swaps it in for `exe`.
fn install(tag: &str, exe: &Path) -> Result<(), String> {
    let name = asset_name().ok_or("no prebuilt binary for this platform")?;
    let sums = get(&format!("{DOWNLOAD_URL}/{tag}/SHA256SUMS"), DOWNLOAD_TIMEOUT).map_err(|e| format!("could not download checksums: {e}"))?;
    let sums = String::from_utf8_lossy(&sums);
    let want = expected_sum(&sums, name).ok_or_else(|| format!("release {tag} has no checksum for {name}"))?;
    let binary = get(&format!("{DOWNLOAD_URL}/{tag}/{name}"), DOWNLOAD_TIMEOUT).map_err(|e| format!("could not download {name}: {e}"))?;
    let got = sha256_hex(&binary);
    if !got.eq_ignore_ascii_case(want) {
        return Err(format!("checksum mismatch for {name} (expected {want}, got {got})"));
    }
    // Written beside the target and renamed over it: the swap is atomic, and replacing
    // a running program's file this way is safe.
    let staged = exe.with_extension("new");
    fs::write(&staged, &binary).and_then(|()| fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))).and_then(|()| fs::rename(&staged, exe)).map_err(
        |e| {
            let _ = fs::remove_file(&staged);
            format!("could not replace {}: {e}", exe.display())
        },
    )
}

/// Installs release `tag` over the running binary.
fn upgrade(tag: &str, exe: &Path) -> Result<(), String> {
    log::info!("updating {VERSION} -> {tag}");
    println!("Updating typeshelf {VERSION} -> {}", tag.trim_start_matches('v'));
    install(tag, exe)?;
    log::info!("update installed at {}", exe.display());
    Ok(())
}

/// Resolved before any replacement; afterwards the running image has no path.
fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("cannot tell where typeshelf is installed: {e}"))
}

/// Called before the app starts. Updates and restarts when a newer release exists;
/// otherwise, or on any failure, returns so the current version runs.
pub fn before_start(settings: &Settings) {
    let Ok(exe) = current_exe() else { return };
    if let Some(reason) = skip_reason(settings, &exe) {
        return log::info!("update check skipped: {reason}");
    }
    let (latest, tag) = match latest_release() {
        Ok(release) => release,
        // Usually just offline: not worth a word on screen.
        Err(e) => return log::info!("update check failed: {e}"),
    };
    if parse_version(VERSION).is_none_or(|current| latest <= current) {
        return log::info!("update check: {VERSION} is current (latest release {tag})");
    }
    match upgrade(&tag, &exe) {
        Ok(()) => {
            let err = Command::new(&exe).args(std::env::args_os().skip(1)).env(SKIP_ENV, "1").exec();
            log::error!("could not restart after update: {err}");
            eprintln!("typeshelf was updated; start it again to use the new version.");
        }
        Err(e) => {
            log::warning!("update failed: {e}");
            eprintln!("typeshelf: update failed ({e}); starting the current version.");
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

/// `typeshelf update`: the same check on demand, reporting what happened.
pub fn command(settings: &Settings) -> Result<(), String> {
    let exe = current_exe()?;
    // Asked for explicitly, so the setting and the environment switch don't apply.
    let forced = Settings { auto_update: true, ..settings.clone() };
    if let Some(reason) = skip_reason(&forced, &exe).filter(|r| !r.starts_with(SKIP_ENV)) {
        return Err(format!("this copy can't update itself: {reason}"));
    }
    let (latest, tag) = latest_release().map_err(|e| format!("could not check for updates: {e}"))?;
    if parse_version(VERSION).is_none_or(|current| latest <= current) {
        println!("typeshelf {VERSION} is up to date (latest release is {tag}).");
        return Ok(());
    }
    upgrade(&tag, &exe)?;
    println!("Updated to {}.", tag.trim_start_matches('v'));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_versions() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.10.3\n"), Some((1, 10, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("v1.2.x"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
        assert!(parse_version(VERSION).is_some());
    }

    #[test]
    fn finds_checksum_by_exact_name() {
        let sums = "aaa  typeshelf-x86_64-unknown-linux-musl\nbbb *typeshelf-aarch64-apple-darwin\n";
        assert_eq!(expected_sum(sums, "typeshelf-aarch64-apple-darwin"), Some("bbb"));
        assert_eq!(expected_sum(sums, "typeshelf-x86_64-unknown-linux-musl"), Some("aaa"));
        assert_eq!(expected_sum(sums, "typeshelf-x86_64"), None);
    }

    #[test]
    fn hashes_like_sha256sum() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
