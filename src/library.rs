//! The book catalog (embedded), on-demand downloads with a disk cache, and the user's
//! own imported texts.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::parse::parse_book;
use crate::store::{cache_dir, data_dir, write_atomic};

const REPO: &str = "mlschmitt/classic-books-markdown";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BookMeta {
    /// Path within the books repo, e.g. "Jane Austen/Pride and Prejudice.md"; for
    /// imported texts, "my/<file>".
    pub path: String,
    pub title: String,
    pub author: String,
    pub year: String,
    pub chars: u32,
    pub chapters: u32,
    #[serde(skip)]
    pub mine: bool,
}

#[derive(Serialize, Deserialize)]
struct CatalogFile {
    /// Commit of the books repo the catalog was built from; downloads are pinned to it.
    rev: String,
    books: Vec<BookMeta>,
}

pub struct Library {
    pub rev: String,
    pub books: Vec<BookMeta>,
    pub books_dir: Option<PathBuf>,
}

fn texts_dir() -> PathBuf {
    data_dir().join("texts")
}

fn meta_from(path: String, md: &str, fallback_title: &str, mine: bool) -> BookMeta {
    let b = parse_book(md, fallback_title);
    BookMeta { path, title: b.title, author: b.author, year: b.year, chars: b.chapters.iter().map(|c| c.length).sum(), chapters: b.chapters.len() as u32, mine }
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Library {
    pub fn load(books_dir: Option<PathBuf>) -> Self {
        let file: CatalogFile = serde_json::from_str(include_str!("../assets/catalog.json")).expect("embedded catalog is valid");
        let mut lib = Library { rev: file.rev, books: file.books, books_dir };
        lib.books.extend(lib.scan_mine());
        lib
    }

    fn scan_mine(&self) -> Vec<BookMeta> {
        let Ok(dir) = fs::read_dir(texts_dir()) else { return vec![] };
        let mut out: Vec<BookMeta> = dir
            .flatten()
            .filter_map(|e| {
                let md = fs::read_to_string(e.path()).ok()?;
                Some(meta_from(format!("my/{}", e.file_name().to_string_lossy()), &md, &stem(&e.path()), true))
            })
            .collect();
        out.sort_by(|a, b| a.title.cmp(&b.title));
        out
    }

    /// Where a book's text lives (or will live) on disk.
    fn local_path(&self, meta: &BookMeta) -> PathBuf {
        match meta.path.strip_prefix("my/") {
            Some(name) if meta.mine => texts_dir().join(name),
            _ => cache_dir().join("books").join(&meta.path),
        }
    }

    pub fn is_local(&self, meta: &BookMeta) -> bool {
        self.local_path(meta).exists() || self.books_dir.as_ref().is_some_and(|d| d.join(&meta.path).exists())
    }

    /// Reads a book from the user's clone or the cache; `None` means it needs downloading.
    pub fn read_local(&self, meta: &BookMeta) -> Option<String> {
        if let Some(dir) = &self.books_dir
            && let Ok(s) = fs::read_to_string(dir.join(&meta.path))
        {
            return Some(s);
        }
        fs::read_to_string(self.local_path(meta)).ok()
    }

    pub fn url(&self, meta: &BookMeta) -> String {
        let path: Vec<String> = meta.path.split('/').map(percent_encode).collect();
        format!("https://raw.githubusercontent.com/{REPO}/{}/{}", self.rev, path.join("/"))
    }

    /// Downloads a book into the cache. Blocking; run it off the UI thread.
    pub fn download(url: &str, dest: &Path) -> Result<String, String> {
        let mut res = ureq::get(url).call().map_err(|e| e.to_string())?;
        let text = res.body_mut().with_config().limit(64 << 20).read_to_string().map_err(|e| e.to_string())?;
        write_atomic(dest, &text).map_err(|e| format!("could not cache book: {e}"))?;
        Ok(text)
    }

    pub fn cache_path(&self, meta: &BookMeta) -> PathBuf {
        self.local_path(meta)
    }

    /// Copies a text or markdown file into the user's texts and adds it to the library.
    pub fn import(&mut self, src: &Path) -> Result<usize, String> {
        let text = fs::read_to_string(src).map_err(|e| format!("{}: {e}", src.display()))?;
        if text.trim().is_empty() {
            return Err(format!("{}: file is empty", src.display()));
        }
        // Keep the original file name as the title; the stored file is named by slug.
        let text = if text.trim_start().starts_with("# Title:") { text } else { format!("# Title: {}\n\n-------\n\n{text}", stem(src)) };
        let name = format!("{}.md", slug(&stem(src)));
        write_atomic(&texts_dir().join(&name), &text).map_err(|e| e.to_string())?;
        let path = format!("my/{name}");
        let meta = meta_from(path.clone(), &text, &stem(src), true);
        match self.books.iter().position(|b| b.mine && b.path == path) {
            Some(i) => {
                self.books[i] = meta;
                Ok(i)
            }
            None => {
                self.books.push(meta);
                Ok(self.books.len() - 1)
            }
        }
    }

    pub fn remove_mine(&mut self, index: usize) -> std::io::Result<()> {
        fs::remove_file(self.local_path(&self.books[index]))?;
        self.books.remove(index);
        Ok(())
    }
}

fn percent_encode(segment: &str) -> String {
    let mut out = String::new();
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn slug(s: &str) -> String {
    let ascii: String = s.nfd().filter(char::is_ascii).collect();
    let mut out = String::new();
    for c in ascii.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "text".into() } else { out }
}

/// Builds `assets/catalog.json` from a clone of the books repo.
pub fn build_catalog(repo: &Path, out: &Path) -> Result<usize, String> {
    let rev = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "master".into());
    let mut books = vec![];
    for author in fs::read_dir(repo).map_err(|e| e.to_string())?.flatten() {
        let name = author.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !author.path().is_dir() {
            continue;
        }
        for file in fs::read_dir(author.path()).map_err(|e| e.to_string())?.flatten() {
            let fname = file.file_name().to_string_lossy().into_owned();
            if !fname.ends_with(".md") {
                continue;
            }
            let md = fs::read_to_string(file.path()).map_err(|e| format!("{fname}: {e}"))?;
            let mut meta = meta_from(format!("{name}/{fname}"), &md, &stem(&file.path()), false);
            if meta.author.is_empty() {
                meta.author = name.clone();
            }
            if meta.chars > 0 {
                books.push(meta);
            }
        }
    }
    books.sort_by_key(|a| a.title.to_lowercase());
    let n = books.len();
    let json = serde_json::to_string(&CatalogFile { rev, books }).map_err(|e| e.to_string())?;
    write_atomic(out, &json.replace("},{", "},\n{")).map_err(|e| e.to_string())?;
    Ok(n)
}
