//! Settings and typing history on disk, under XDG-style directories on every platform.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const APP: &str = "typeshelf";

fn base(var: &str, home_rel: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join(APP),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(home_rel).join(APP),
    }
}

pub fn config_dir() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config")
}
pub fn data_dir() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share")
}
pub fn cache_dir() -> PathBuf {
    base("XDG_CACHE_HOME", ".cache")
}
pub fn state_dir() -> PathBuf {
    base("XDG_STATE_HOME", ".local/state")
}

/// The file this program really is, with symlinks resolved. `current_exe` alone resolves
/// them on Linux but on macOS returns the path the program was started by, e.g. Homebrew's
/// `bin/typeshelf` link instead of the file in its `Cellar`.
pub fn real_exe() -> std::io::Result<PathBuf> {
    std::env::current_exe().and_then(fs::canonicalize)
}

/// The source checkout this binary was built in, when it is being run from that
/// checkout's `target/` directory (directly or through a symlink).
pub fn checkout_root() -> Option<PathBuf> {
    let exe = real_exe().ok()?;
    let target = exe.parent()?.parent()?;
    let root = target.parent()?;
    (target.file_name()? == "target" && root.join("Cargo.toml").exists()).then(|| root.to_path_buf())
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Writes via a temp file so a crash mid-write can't truncate the real one.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(tmp, path)
}

fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    write_atomic(path, &serde_json::to_string_pretty(value).unwrap_or_default())
}

pub const PAGE_LENGTHS: [(&str, usize); 4] = [("short", 450), ("medium", 800), ("long", 1300), ("very long", 2200)];
pub const TEXT_WIDTHS: [u16; 5] = [50, 60, 70, 80, 100];
pub const CURSORS: [&str; 4] = ["terminal", "block", "bar", "underline"];

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub page_length: String,
    pub text_width: u16,
    pub cursor: String,
    pub stop_on_error: bool,
    pub space_for_enter: bool,
    pub live_stats: bool,
    /// Check GitHub for a newer release at startup and install it.
    pub auto_update: bool,
    /// A local clone of classic-books-markdown, read instead of downloading.
    pub books_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "terminal".into(),
            page_length: "medium".into(),
            text_width: 70,
            cursor: "terminal".into(),
            stop_on_error: false,
            space_for_enter: true,
            live_stats: true,
            auto_update: true,
            books_dir: None,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        load(&config_dir().join("config.json"))
    }
    pub fn save(&self) -> std::io::Result<()> {
        save(&config_dir().join("config.json"), self)
    }
    pub fn page_target(&self) -> usize {
        PAGE_LENGTHS.iter().find(|p| p.0 == self.page_length).map_or(800, |p| p.1)
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct BookProgress {
    /// Length of each chapter, so progress can be shown without loading the book.
    pub lens: Vec<u32>,
    /// Furthest point reached in each chapter; what the progress bars show.
    pub off: Vec<u32>,
    /// Where the cursor was last left in each chapter; where it reopens. Differs from
    /// `off` when a chapter is being typed again.
    #[serde(default)]
    pub cur: Vec<u32>,
    pub last_ch: usize,
    pub last_at: u64,
}

impl BookProgress {
    pub fn fraction(&self) -> f64 {
        let total: u64 = self.lens.iter().map(|&n| n as u64).sum();
        let done: u64 = self.lens.iter().zip(&self.off).map(|(&l, &o)| o.min(l) as u64).sum();
        if total == 0 { 0.0 } else { done as f64 / total as f64 }
    }
    pub fn chapter_fraction(&self, ch: usize) -> f64 {
        match (self.lens.get(ch), self.off.get(ch)) {
            (Some(&l), Some(&o)) if l > 0 => o.min(l) as f64 / l as f64,
            _ => 0.0,
        }
    }
    pub fn finished(&self) -> bool {
        !self.lens.is_empty() && self.lens.iter().zip(&self.off).all(|(l, o)| o >= l)
    }
}

/// One completed page.
#[derive(Serialize, Deserialize, Clone)]
pub struct Record {
    pub book: String,
    pub title: String,
    pub ch: usize,
    pub at: u64,
    pub wpm: f32,
    pub acc: f32,
    pub chars: u32,
    pub ms: u64,
    pub mistakes: u32,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct State {
    pub progress: HashMap<String, BookProgress>,
    pub history: Vec<Record>,
    /// Mistakes per expected key, over all time.
    pub missed: HashMap<String, u32>,
}

impl State {
    pub fn load() -> Self {
        load(&data_dir().join("state.json"))
    }
    pub fn save(&self) -> std::io::Result<()> {
        save(&data_dir().join("state.json"), self)
    }

    /// Level and progress towards the next one (0..1). Level `n` is reached after
    /// typing `500 * n * (n - 1)` characters, so each level takes a little longer.
    pub fn level(&self) -> (u32, f64) {
        let chars: u64 = self.history.iter().map(|r| r.chars as u64).sum();
        let at = |n: u64| 500 * n * (n - 1);
        let mut level = 1;
        while chars >= at(level + 1) {
            level += 1;
        }
        (level as u32, (chars - at(level)) as f64 / (at(level + 1) - at(level)) as f64)
    }

    /// Average speed and accuracy over the pages typed in a book, or one chapter of it,
    /// weighted by page size, with the page count.
    pub fn averages(&self, book: &str, ch: Option<usize>) -> Option<(f32, f32, usize)> {
        let pages: Vec<&Record> = self.history.iter().filter(|r| r.book == book && ch.is_none_or(|c| r.ch == c)).collect();
        let chars: f32 = pages.iter().map(|r| r.chars as f32).sum();
        let ms: f32 = pages.iter().map(|r| r.ms as f32).sum();
        if chars == 0.0 || ms == 0.0 {
            return None;
        }
        let acc = pages.iter().map(|r| r.acc * r.chars as f32).sum::<f32>() / chars;
        let wpm = pages.iter().map(|r| r.wpm * r.ms as f32).sum::<f32>() / ms;
        Some((wpm, acc, pages.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(chars: u32, wpm: f32, ch: usize) -> Record {
        Record { book: "b".into(), title: "B".into(), ch, at: 0, wpm, acc: 100.0, chars, ms: 60_000, mistakes: 0 }
    }

    #[test]
    fn levels_grow_with_characters_typed() {
        let mut s = State::default();
        assert_eq!(s.level(), (1, 0.0));
        s.history.push(rec(500, 50.0, 0));
        assert_eq!(s.level(), (1, 0.5));
        s.history.push(rec(2500, 50.0, 0));
        assert_eq!(s.level().0, 3);
    }

    #[test]
    fn averages_by_book_and_chapter() {
        let mut s = State::default();
        s.history.extend([rec(100, 40.0, 0), rec(100, 60.0, 1)]);
        assert_eq!(s.averages("b", None).map(|a| (a.0, a.2)), Some((50.0, 2)));
        assert_eq!(s.averages("b", Some(1)).map(|a| a.0), Some(60.0));
        assert!(s.averages("other", None).is_none());
    }
}
