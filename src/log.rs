//! A plain-text trace of what the app did, for diagnosing problems after the fact.
//! Records events and their outcomes - never the keys typed.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Local;

use crate::store::{checkout_root, state_dir};

/// The log is rotated to `<name>.1` at startup once it passes this size.
const MAX_BYTES: u64 = 1 << 20;

static LOG: Mutex<Option<File>> = Mutex::new(None);

/// `TYPESHELF_LOG` if set; `logs/` in the checkout when running a build from
/// `target/`, so development traces sit beside the code; otherwise the state directory.
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("TYPESHELF_LOG").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    match checkout_root() {
        Some(root) => root.join("logs").join("typeshelf.log"),
        None => state_dir().join("typeshelf.log"),
    }
}

/// Opens the log for appending. Logging is best effort: if the file can't be opened the
/// app runs without it.
pub fn init() {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
    }
}

pub fn write(level: &str, msg: std::fmt::Arguments) {
    // A poisoned lock still holds a usable file, and the panic hook logs through here.
    let mut guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(file) = guard.as_mut() {
        let _ = writeln!(file, "{} {level:<5} {msg}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"));
    }
}

macro_rules! info {
    ($($arg:tt)*) => { $crate::log::write("INFO", format_args!($($arg)*)) };
}
macro_rules! warning {
    ($($arg:tt)*) => { $crate::log::write("WARN", format_args!($($arg)*)) };
}
macro_rules! error {
    ($($arg:tt)*) => { $crate::log::write("ERROR", format_args!($($arg)*)) };
}
pub(crate) use {error, info, warning};
