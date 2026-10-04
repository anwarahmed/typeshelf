//! Self-update: at startup, compare the commit this binary was built from with the
//! latest on GitHub and, if they differ, rebuild from source and restart.
//!
//! There are no prebuilt binaries, so an update is a fresh clone built by `install.sh`.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::log;
use crate::store::{Settings, cache_dir, checkout_root};

/// The commit this binary was built from; empty if unknown, `-dirty` if the tree had
/// local changes.
pub const COMMIT: &str = env!("TYPESHELF_COMMIT");

const REPO_URL: &str = "https://github.com/anwarahmed/typeshelf.git";
const LATEST_URL: &str = "https://api.github.com/repos/anwarahmed/typeshelf/commits/main";
/// Set on the restarted process so a failed or raced update can't loop.
const SKIP_ENV: &str = "TYPESHELF_NO_UPDATE";
/// Starting the app must not hang on a bad connection.
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

/// Why this build doesn't update itself, if it doesn't.
fn skip_reason(settings: &Settings) -> Option<&'static str> {
    if std::env::var_os(SKIP_ENV).is_some_and(|v| !v.is_empty()) {
        Some("TYPESHELF_NO_UPDATE is set")
    } else if !settings.auto_update {
        Some("turned off in settings")
    } else if COMMIT.is_empty() {
        Some("this build doesn't know which commit it came from")
    } else if COMMIT.ends_with("-dirty") {
        Some("this build has local changes")
    } else if checkout_root().is_some() {
        Some("running from a source checkout; use git pull and cargo build")
    } else {
        None
    }
}

fn latest_commit() -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(CHECK_TIMEOUT)).build().into();
    let mut res = agent.get(LATEST_URL).header("Accept", "application/vnd.github.sha").header("User-Agent", "typeshelf").call().map_err(|e| e.to_string())?;
    let sha = res.body_mut().read_to_string().map_err(|e| e.to_string())?.trim().to_string();
    if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) { Ok(sha) } else { Err("unexpected reply from GitHub".into()) }
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    match cmd.status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{what} failed ({s})")),
        Err(e) => Err(format!("could not run {what}: {e}")),
    }
}

/// Clones the repo and lets its `install.sh` build and replace the binary at `exe`.
fn install(exe: &Path) -> Result<(), String> {
    let bin_dir = exe.parent().ok_or("cannot tell where typeshelf is installed")?;
    let src = cache_dir().join("update");
    let _ = std::fs::remove_dir_all(&src);
    let result = run(Command::new("git").args(["clone", "--quiet", "--depth", "1", REPO_URL]).arg(&src), "git clone")
        .and_then(|()| run(Command::new("sh").arg(src.join("install.sh")).env("TYPESHELF_BIN_DIR", bin_dir), "install.sh"));
    let _ = std::fs::remove_dir_all(&src);
    result
}

/// Installs `latest` over the running binary. Returns the binary's path, resolved
/// before the file is replaced (afterwards the running image has no path).
fn upgrade(latest: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    log::info!("updating {} -> {}", short(COMMIT), short(latest));
    println!("Updating typeshelf ({} -> {}). This takes a minute or two.", short(COMMIT), short(latest));
    install(&exe)?;
    log::info!("update installed at {}", exe.display());
    Ok(exe)
}

/// Called before the app starts. Updates and restarts when a newer version exists;
/// otherwise, or on any failure, returns so the current version runs.
pub fn before_start(settings: &Settings) {
    if let Some(reason) = skip_reason(settings) {
        log::info!("update check skipped: {reason}");
        return;
    }
    let latest = match latest_commit() {
        Ok(latest) => latest,
        // Usually just offline: not worth a word on screen.
        Err(e) => return log::info!("update check failed: {e}"),
    };
    if latest == COMMIT {
        return log::info!("update check: up to date at {}", short(COMMIT));
    }
    match upgrade(&latest) {
        Ok(exe) => {
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
    if let Some(reason) = skip_reason(&Settings { auto_update: true, ..settings.clone() }) {
        return Err(format!("this copy can't update itself: {reason}"));
    }
    let latest = latest_commit().map_err(|e| format!("could not reach GitHub: {e}"))?;
    if latest == COMMIT {
        println!("typeshelf is up to date ({}).", short(COMMIT));
        return Ok(());
    }
    upgrade(&latest).map(|_| ())
}
