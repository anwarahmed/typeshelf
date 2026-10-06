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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::log;
use crate::store::{Settings, checkout_root, real_exe, state_dir};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit this binary was built from, for `--version` and the log; empty if
/// unknown, `-dirty` if the tree had local changes.
pub const COMMIT: &str = env!("TYPESHELF_COMMIT");

/// Where the latest release's files are: `VERSION`, `SHA256SUMS` and one binary per
/// platform. Plain downloads, not the GitHub API: the API allows 60 requests an hour
/// per address without a token, which a check on every start can use up on a shared
/// network, and the check then fails silently.
const RELEASES: &str = "https://github.com/anwarahmed/typeshelf/releases/latest/download";
/// Points the updater somewhere else, as it does `install.sh`; `file://` works, which
/// is how the updater is tested.
const URL_ENV: &str = "TYPESHELF_RELEASE_URL";
/// A package that owns its copy installs this file, relative to the directory of the
/// binary, naming itself and how to upgrade. Homebrew's formula and the AUR package
/// both do; typeshelf then leaves updating to them.
const MANAGED_BY: &str = "../share/typeshelf/managed-by";
/// Set on the restarted process so a failed or raced update can't loop.
const SKIP_ENV: &str = "TYPESHELF_NO_UPDATE";
/// The check at startup looks for a release at most this often. It costs a network
/// round trip before the app appears, and releases are rare; `typeshelf update` checks at
/// once regardless.
const CHECK_EVERY: u64 = 24 * 60 * 60;
/// Holds the time of the last check that got an answer, in seconds since 1970, in the
/// state directory.
const STAMP: &str = "last-update-check";
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

/// The package that owns the copy at `exe`, as its marker file words it
/// ("Homebrew; use brew upgrade typeshelf"), if one does.
fn managed_by(exe: &Path) -> Option<String> {
    let marker = fs::read_to_string(exe.parent()?.join(MANAGED_BY)).ok()?;
    marker.lines().next().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string)
}

/// Why this copy doesn't update itself, if it doesn't.
fn skip_reason(settings: &Settings, exe: &Path) -> Option<String> {
    let fixed = |s: &str| Some(s.to_string());
    if std::env::var_os(SKIP_ENV).is_some_and(|v| !v.is_empty()) {
        fixed("TYPESHELF_NO_UPDATE is set")
    } else if !settings.auto_update {
        fixed("turned off in settings")
    } else if checkout_root().is_some() {
        fixed("running from a source checkout; use git pull and cargo build")
    } else if let Some(owner) = managed_by(exe) {
        // The package said so itself when it installed this copy; nothing is guessed.
        Some(format!("installed with {owner}"))
    } else if exe.components().any(|c| c.as_os_str() == "Cellar") {
        // In case a formula ever ships without the marker. `exe` has symlinks resolved,
        // so this sees through the link Homebrew puts in its bin directory.
        fixed("installed with Homebrew; use brew upgrade typeshelf")
    } else if asset_name().is_none() {
        fixed("no prebuilt binary for this platform; rebuild from source to update")
    } else if !exe.parent().is_some_and(writable) {
        fixed("its directory is not writable, so a package manager probably owns it; update it the way you installed it")
    } else {
        None
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether a check that got an answer at `last` is still fresh at `now`. A time in the
/// future (the clock was set back) does not count: better one check too many than none
/// for however long the clock was wrong by.
fn fresh(last: u64, now: u64) -> bool {
    now >= last && now - last < CHECK_EVERY
}

fn checked_recently() -> bool {
    fs::read_to_string(state_dir().join(STAMP)).ok().and_then(|s| s.trim().parse().ok()).is_some_and(|last| fresh(last, now_secs()))
}

/// Notes that the release server answered just now. Only an answer is noted: a check
/// that failed (offline, usually) is tried again at the next start.
fn record_check() {
    let dir = state_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join(STAMP), format!("{}\n", now_secs()));
}

fn base() -> String {
    std::env::var(URL_ENV).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| RELEASES.to_string())
}

fn get(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read(path).map_err(|e| e.to_string());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(timeout)).build().into();
    let mut res = agent.get(url).header("User-Agent", "typeshelf").call().map_err(|e| e.to_string())?;
    res.body_mut().with_config().limit(64 << 20).read_to_vec().map_err(|e| e.to_string())
}

/// The newest released version, as numbers and as a tag. Read from the `VERSION` file
/// each release carries (since 0.2.5).
fn latest_release() -> Result<(Version, String), String> {
    let body = get(&format!("{}/VERSION", base()), CHECK_TIMEOUT)?;
    let text = String::from_utf8_lossy(&body).trim().trim_start_matches('v').to_string();
    Ok((parse_version(&text).ok_or_else(|| format!("unrecognized release version {text:?}"))?, format!("v{text}")))
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

/// Downloads the latest release's binary for this platform, checks it against the
/// release's checksums, and swaps it in for `exe`.
fn install(tag: &str, exe: &Path) -> Result<(), String> {
    let name = asset_name().ok_or("no prebuilt binary for this platform")?;
    let base = base();
    let sums = get(&format!("{base}/SHA256SUMS"), DOWNLOAD_TIMEOUT).map_err(|e| format!("could not download checksums: {e}"))?;
    let sums = String::from_utf8_lossy(&sums);
    let want = expected_sum(&sums, name).ok_or_else(|| format!("release {tag} has no checksum for {name}"))?;
    let binary = get(&format!("{base}/{name}"), DOWNLOAD_TIMEOUT).map_err(|e| format!("could not download {name}: {e}"))?;
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

/// Resolved before any replacement; afterwards the running image has no path. The real
/// file, not a link to it: the checks in `skip_reason` are about where it is installed,
/// and replacing a link would leave the installed file behind (and broke `brew upgrade`).
fn current_exe() -> Result<PathBuf, String> {
    real_exe().map_err(|e| format!("cannot tell where typeshelf is installed: {e}"))
}

/// Called before the app starts. Updates and restarts when a newer release exists;
/// otherwise, or on any failure, returns so the current version runs.
pub fn before_start(settings: &Settings) {
    let Ok(exe) = current_exe() else { return };
    if let Some(reason) = skip_reason(settings, &exe) {
        return log::info!("update check skipped: {reason}");
    }
    if checked_recently() {
        return log::info!("update check skipped: already checked in the last day");
    }
    let (latest, tag) = match latest_release() {
        Ok(release) => release,
        // Usually just offline: not worth a word on screen.
        Err(e) => return log::info!("update check failed: {e}"),
    };
    if parse_version(VERSION).is_none_or(|current| latest <= current) {
        record_check();
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
        record_check();
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
    fn a_marker_file_names_the_package_that_owns_a_copy() {
        let root = std::env::temp_dir().join(format!("typeshelf-test-{}-managed", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).unwrap();
        let exe = root.join("bin/typeshelf");
        assert_eq!(managed_by(&exe), None);
        fs::create_dir_all(root.join("share/typeshelf")).unwrap();
        fs::write(root.join("share/typeshelf/managed-by"), "Homebrew; use brew upgrade typeshelf\n").unwrap();
        assert_eq!(managed_by(&exe).as_deref(), Some("Homebrew; use brew upgrade typeshelf"));
        // The marker outranks every guess about the path.
        let settings = Settings { auto_update: true, ..Settings::default() };
        if std::env::var_os(SKIP_ENV).is_none() {
            assert_eq!(skip_reason(&settings, &exe).as_deref(), Some("installed with Homebrew; use brew upgrade typeshelf"));
        }
        fs::write(root.join("share/typeshelf/managed-by"), "\n").unwrap();
        assert_eq!(managed_by(&exe), None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reads_a_release_from_a_directory() {
        // What the end-to-end test relies on: file:// is read from disk.
        let dir = std::env::temp_dir().join(format!("typeshelf-test-{}-get", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("VERSION"), "1.2.3\n").unwrap();
        assert_eq!(get(&format!("file://{}/VERSION", dir.display()), CHECK_TIMEOUT).unwrap(), b"1.2.3\n");
        assert!(get(&format!("file://{}/missing", dir.display()), CHECK_TIMEOUT).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn checks_at_most_once_a_day() {
        let day = CHECK_EVERY;
        assert!(fresh(1000, 1000));
        assert!(fresh(1000, 1000 + day - 1));
        assert!(!fresh(1000, 1000 + day));
        // Never checked, or the clock went backwards: check.
        assert!(!fresh(0, 2 * day));
        assert!(!fresh(5000, 1000));
    }

    #[test]
    fn hashes_like_sha256sum() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
