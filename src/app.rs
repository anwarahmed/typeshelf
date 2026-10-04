//! Application state and key handling. Drawing lives in `ui`.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::engine::{Session, Stats};
use crate::library::Library;
use crate::log;
use crate::paginate::{Page, paginate};
use crate::parse::{Book, parse_book};
use crate::store::{BookProgress, CURSORS, PAGE_LENGTHS, Record, Settings, State, TEXT_WIDTHS, now_secs};
use crate::theme::{self, THEMES, Theme};

#[derive(Clone, Copy, PartialEq)]
pub enum Screen {
    Library,
    Book,
    Typing,
    Stats,
    Settings,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    All,
    Reading,
    Finished,
    Mine,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::All, Tab::Reading, Tab::Finished, Tab::Mine];
    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "All",
            Tab::Reading => "Reading",
            Tab::Finished => "Finished",
            Tab::Mine => "My texts",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Sort {
    Title,
    Author,
    Year,
    Length,
    Recent,
}

impl Sort {
    const ALL: [Sort; 5] = [Sort::Title, Sort::Author, Sort::Year, Sort::Length, Sort::Recent];
    pub fn label(self) -> &'static str {
        match self {
            Sort::Title => "title",
            Sort::Author => "author",
            Sort::Year => "year",
            Sort::Length => "length",
            Sort::Recent => "recent",
        }
    }
}

pub enum Confirm {
    ResetBook,
    DeleteMine(usize),
}

pub struct OpenBook {
    /// Index into `Library::books`.
    pub index: usize,
    pub book: Book,
    /// Page count of each chapter at the current page length.
    pub pages: Vec<usize>,
    pub sel: usize,
}

pub struct Typing {
    pub ch: usize,
    pub pages: Vec<Page>,
    pub page: usize,
    pub session: Session,
    /// Set once the page is complete; the result panel shows it.
    pub result: Option<Stats>,
}

pub const SETTING_LABELS: [&str; 8] =
    ["Theme", "Page length", "Text width", "Cursor", "Stop on error", "Space types line breaks", "Live stats while typing", "Update on start"];

enum Msg {
    Downloaded(usize, Result<String, String>),
}

pub struct App {
    pub settings: Settings,
    pub state: State,
    pub lib: Library,
    pub theme: Theme,
    pub screen: Screen,

    pub tab: Tab,
    pub sort: Sort,
    pub query: String,
    pub searching: bool,
    /// Indices into `Library::books` matching the tab and query, sorted.
    pub filtered: Vec<usize>,
    pub lib_sel: usize,

    pub open: Option<OpenBook>,
    pub typing: Option<Typing>,
    pub settings_sel: usize,

    pub help: bool,
    pub confirm: Option<Confirm>,
    /// Index of the book being downloaded.
    pub loading: Option<usize>,
    pub toast: Option<(String, Instant)>,
    pub quit: bool,

    clock: Instant,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

fn step(len: usize, cur: usize, delta: isize) -> usize {
    if len == 0 { 0 } else { (cur as isize + delta).clamp(0, len as isize - 1) as usize }
}

fn cycle<T: PartialEq + Copy>(all: &[T], cur: T, delta: isize) -> T {
    let i = all.iter().position(|x| *x == cur).unwrap_or(0) as isize;
    all[(i + delta).rem_euclid(all.len() as isize) as usize]
}

impl App {
    pub fn new(settings: Settings, state: State, lib: Library) -> Self {
        let (tx, rx) = channel();
        let mut app = App {
            theme: theme::by_name(&settings.theme),
            settings,
            state,
            lib,
            screen: Screen::Library,
            tab: Tab::All,
            sort: Sort::Title,
            query: String::new(),
            searching: false,
            filtered: vec![],
            lib_sel: 0,
            open: None,
            typing: None,
            settings_sel: 0,
            help: false,
            confirm: None,
            loading: None,
            toast: None,
            quit: false,
            clock: Instant::now(),
            tx,
            rx,
        };
        app.refilter();
        app
    }

    pub fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    /// Shows a message in the footer. Everything the user is told is also logged.
    pub fn toast(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::warning!("toast: {msg}");
        self.toast = Some((msg, Instant::now()));
    }

    fn save_state(&mut self) {
        if let Err(e) = self.state.save() {
            self.toast(format!("Could not save progress: {e}"));
        }
    }

    /// Called every loop iteration: collects finished downloads, expires toasts.
    pub fn tick(&mut self) {
        while let Ok(Msg::Downloaded(index, res)) = self.rx.try_recv() {
            if self.loading != Some(index) {
                continue;
            }
            self.loading = None;
            match res {
                Ok(text) => {
                    log::info!("downloaded {:?} ({} bytes)", self.lib.books[index].path, text.len());
                    self.show_book(index, &text)
                }
                Err(e) => self.toast(format!("Download failed: {e}")),
            }
        }
        if self.toast.as_ref().is_some_and(|t| t.1.elapsed().as_secs() >= 4) {
            self.toast = None;
        }
    }

    // ---- library ----

    pub fn progress_of(&self, index: usize) -> Option<&BookProgress> {
        self.state.progress.get(&self.lib.books[index].path)
    }

    pub fn refilter(&mut self) {
        let terms: Vec<String> = self.query.to_lowercase().split_whitespace().map(String::from).collect();
        let mut out: Vec<usize> = (0..self.lib.books.len())
            .filter(|&i| {
                let b = &self.lib.books[i];
                let p = self.progress_of(i);
                let in_tab = match self.tab {
                    Tab::All => true,
                    Tab::Reading => p.is_some_and(|p| !p.finished()),
                    Tab::Finished => p.is_some_and(|p| p.finished()),
                    Tab::Mine => b.mine,
                };
                in_tab && {
                    let hay = format!("{} {}", b.title, b.author).to_lowercase();
                    terms.iter().all(|t| hay.contains(t))
                }
            })
            .collect();
        let books = &self.lib.books;
        let year = |i: usize| {
            books[i]
                .year
                .trim_start_matches(|c: char| !c.is_ascii_digit())
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|y| y.parse::<i32>().ok())
                .map(|y| if books[i].year.contains("BC") { -y } else { y })
        };
        let title = |i: usize| books[i].title.to_lowercase();
        match self.sort {
            Sort::Title => out.sort_by_key(|&i| title(i)),
            Sort::Author => out.sort_by_key(|&i| (books[i].author.to_lowercase(), title(i))),
            Sort::Year => out.sort_by_key(|&i| (year(i).unwrap_or(i32::MAX), title(i))),
            Sort::Length => out.sort_by_key(|&i| (books[i].chars, title(i))),
            Sort::Recent => out.sort_by_key(|&i| (std::cmp::Reverse(self.progress_of(i).map_or(0, |p| p.last_at)), title(i))),
        }
        self.filtered = out;
        self.lib_sel = 0;
    }

    pub fn selected_book(&self) -> Option<usize> {
        self.filtered.get(self.lib_sel).copied()
    }

    pub fn open_book(&mut self, index: usize) {
        let meta = &self.lib.books[index];
        if let Some(text) = self.lib.read_local(meta) {
            self.show_book(index, &text);
            return;
        }
        let (url, dest, tx) = (self.lib.url(meta), self.lib.cache_path(meta), self.tx.clone());
        log::info!("downloading {url}");
        self.loading = Some(index);
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Downloaded(index, Library::download(&url, &dest)));
        });
    }

    fn show_book(&mut self, index: usize, text: &str) {
        let meta = &self.lib.books[index];
        let started = Instant::now();
        let book = parse_book(text, &meta.title);
        log::info!(
            "opened {:?}: {} chapters, {} bytes, parsed in {:?}; saved progress {:?}",
            meta.path,
            book.chapters.len(),
            text.len(),
            started.elapsed(),
            self.state.progress.get(&meta.path).map(|p| (p.last_ch, p.fraction()))
        );
        if book.chapters.is_empty() {
            self.toast("This book has no readable text.");
            return;
        }
        let sel = self.state.progress.get(&meta.path).map_or(0, |p| p.last_ch.min(book.chapters.len() - 1));
        let target = self.settings.page_target();
        let pages = book.chapters.iter().map(|c| paginate(&c.lines, target).len()).collect();
        self.open = Some(OpenBook { index, book, pages, sel });
        self.screen = Screen::Book;
    }

    // ---- typing ----

    fn book_key(&self) -> Option<String> {
        self.open.as_ref().map(|o| self.lib.books[o.index].path.clone())
    }

    /// First chapter with something left to type.
    fn resume_chapter(&self) -> usize {
        let Some(open) = &self.open else { return 0 };
        let p = self.book_key().and_then(|k| self.state.progress.get(&k));
        (0..open.book.chapters.len()).find(|&c| p.is_none_or(|p| p.chapter_fraction(c) < 1.0)).unwrap_or(0)
    }

    /// Opens a chapter for typing, at `page` or the first page not yet completed.
    pub fn start_typing(&mut self, ch: usize, page: Option<usize>) {
        let Some(open) = &self.open else { return };
        let pages = paginate(&open.book.chapters[ch].lines, self.settings.page_target());
        if pages.is_empty() {
            return;
        }
        // A cursor at the chapter's end matches no page, so a finished chapter reopens at its top.
        let done = self.book_key().and_then(|k| self.state.progress.get(&k)).and_then(|p| p.cur.get(ch).or(p.off.get(ch)).copied()).unwrap_or(0);
        let page = page.unwrap_or_else(|| pages.iter().position(|p| p.end > done).unwrap_or(0)).min(pages.len() - 1);
        let mut session = self.session_for(&pages[page]);
        // Pick up mid-page where an earlier sitting stopped.
        let start = if page > 0 { pages[page - 1].end } else { 0 };
        if done > start && done < pages[page].end {
            session.resume_at((done - start) as usize);
        }
        log::info!(
            "typing chapter {ch} page {}/{}: saved cursor {done}, page spans {start}..{}, resumed at cell {} of {}",
            page + 1,
            pages.len(),
            pages[page].end,
            session.resumed,
            session.cells.len()
        );
        self.typing = Some(Typing { ch, pages, page, session, result: None });
        self.screen = Screen::Typing;
    }

    fn session_for(&self, page: &Page) -> Session {
        let text = page.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        Session::new(&text, self.settings.stop_on_error, self.settings.space_for_enter)
    }

    fn goto_page(&mut self, page: usize) {
        let Some(t) = &self.typing else { return };
        let session = self.session_for(&t.pages[page]);
        let t = self.typing.as_mut().unwrap();
        t.page = page;
        t.session = session;
        t.result = None;
    }

    fn next_page(&mut self) {
        let Some(t) = &self.typing else { return };
        let chapters = self.open.as_ref().map_or(0, |o| o.book.chapters.len());
        if t.page + 1 < t.pages.len() {
            self.goto_page(t.page + 1);
        } else if t.ch + 1 < chapters {
            let ch = t.ch + 1;
            if let Some(o) = &mut self.open {
                o.sel = ch;
            }
            self.start_typing(ch, Some(0));
        } else {
            self.typing = None;
            self.screen = Screen::Book;
            self.toast("You reached the end of the book.");
        }
    }

    fn prev_page(&mut self) {
        let Some(t) = &self.typing else { return };
        if t.page > 0 {
            self.goto_page(t.page - 1);
        } else if t.ch > 0 {
            let ch = t.ch - 1;
            self.start_typing(ch, Some(usize::MAX));
        }
    }

    /// Sets where the cursor is in a chapter, and notes it as the last one typed. The
    /// furthest point reached only ever moves forward.
    fn record_progress(state: &mut State, key: &str, book: &Book, ch: usize, offset: u32) {
        let lens: Vec<u32> = book.chapters.iter().map(|c| c.length).collect();
        let p = state.progress.entry(key.to_string()).or_default();
        if p.lens != lens {
            p.off.resize(lens.len(), 0);
            p.lens = lens;
        }
        // States saved before `cur` existed only have the furthest point.
        if p.cur.len() != p.off.len() {
            p.cur = p.off.clone();
        }
        p.cur[ch] = offset;
        p.off[ch] = p.off[ch].max(offset);
        p.last_ch = ch;
        p.last_at = now_secs();
    }

    /// Remembers how far into an unfinished page the cursor got, so the chapter reopens there.
    fn save_partial(&mut self) {
        let Some(key) = self.book_key() else { return };
        let (Some(t), Some(open)) = (&self.typing, &self.open) else { return };
        if t.result.is_some() || t.session.pos <= t.session.resumed {
            return;
        }
        let start = if t.page > 0 { t.pages[t.page - 1].end } else { 0 };
        log::info!(
            "left chapter {} page {} at cell {} of {} (chapter offset {})",
            t.ch,
            t.page + 1,
            t.session.pos,
            t.session.cells.len(),
            start + t.session.pos as u32
        );
        Self::record_progress(&mut self.state, &key, &open.book, t.ch, start + t.session.pos as u32);
        self.save_state();
    }

    fn complete_page(&mut self) {
        let now = self.now_ms();
        let Some(key) = self.book_key() else { return };
        let (Some(t), Some(open)) = (&mut self.typing, &self.open) else { return };
        let stats = t.session.stats(now);
        t.result = Some(stats);
        log::info!(
            "completed chapter {} page {}/{}: {:.1} wpm, {:.1}% accuracy, {} chars, {} ms, {} mistakes",
            t.ch,
            t.page + 1,
            t.pages.len(),
            stats.wpm,
            stats.acc,
            stats.chars,
            stats.ms,
            stats.mistakes
        );
        Self::record_progress(&mut self.state, &key, &open.book, t.ch, t.pages[t.page].end);
        for (k, n) in &t.session.missed {
            *self.state.missed.entry(k.to_string()).or_default() += n;
        }
        self.state.history.push(Record {
            book: key,
            title: open.book.title.clone(),
            ch: t.ch,
            at: now_secs(),
            wpm: stats.wpm,
            acc: stats.acc,
            chars: stats.chars,
            ms: stats.ms,
            mistakes: stats.mistakes,
        });
        self.save_state();
    }

    // ---- settings ----

    pub fn setting_value(&self, i: usize) -> String {
        let s = &self.settings;
        let on_off = |b: bool| if b { "on" } else { "off" }.to_string();
        match i {
            0 => s.theme.clone(),
            1 => s.page_length.clone(),
            2 => format!("{} columns", s.text_width),
            3 => s.cursor.clone(),
            4 => on_off(s.stop_on_error),
            5 => on_off(s.space_for_enter),
            6 => on_off(s.live_stats),
            _ => on_off(s.auto_update),
        }
    }

    fn change_setting(&mut self, i: usize, delta: isize) {
        let s = &mut self.settings;
        match i {
            0 => {
                let names: Vec<&str> = THEMES.iter().map(|t| t.name).collect();
                s.theme = cycle(&names, s.theme.as_str(), delta).to_string();
                self.theme = theme::by_name(&s.theme);
            }
            1 => {
                let names: Vec<&str> = PAGE_LENGTHS.iter().map(|p| p.0).collect();
                s.page_length = cycle(&names, s.page_length.as_str(), delta).to_string();
                let target = s.page_target();
                if let Some(o) = &mut self.open {
                    o.pages = o.book.chapters.iter().map(|c| paginate(&c.lines, target).len()).collect();
                }
            }
            2 => s.text_width = cycle(&TEXT_WIDTHS, s.text_width, delta),
            3 => s.cursor = cycle(&CURSORS, s.cursor.as_str(), delta).to_string(),
            4 => s.stop_on_error = !s.stop_on_error,
            5 => s.space_for_enter = !s.space_for_enter,
            6 => s.live_stats = !s.live_stats,
            _ => s.auto_update = !s.auto_update,
        }
        log::info!("setting {:?} = {}", SETTING_LABELS[i.min(SETTING_LABELS.len() - 1)], self.setting_value(i));
        if let Err(e) = self.settings.save() {
            self.toast(format!("Could not save settings: {e}"));
        }
    }

    // ---- keys ----

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.save_partial();
            self.quit = true;
            return;
        }
        self.toast = None;
        if self.help {
            self.help = false;
            return;
        }
        if let Some(confirm) = self.confirm.take() {
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) {
                self.confirmed(confirm);
            }
            return;
        }
        if self.loading.is_some() {
            if key.code == KeyCode::Esc {
                self.loading = None;
            }
            return;
        }
        match self.screen {
            Screen::Typing => self.key_typing(key, ctrl),
            Screen::Library if self.searching => self.key_search(key, ctrl),
            _ => match key.code {
                KeyCode::Char('?') => self.help = true,
                KeyCode::Char('1') => self.screen = Screen::Library,
                KeyCode::Char('2') => self.screen = Screen::Stats,
                KeyCode::Char('3') => self.screen = Screen::Settings,
                _ => match self.screen {
                    Screen::Library => self.key_library(key, ctrl),
                    Screen::Book => self.key_book(key, ctrl),
                    Screen::Settings => self.key_settings(key),
                    _ => {
                        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Left) {
                            self.screen = Screen::Library
                        }
                    }
                },
            },
        }
    }

    fn confirmed(&mut self, confirm: Confirm) {
        match confirm {
            Confirm::ResetBook => {
                if let Some(key) = self.book_key() {
                    log::info!("reset progress for {key:?}");
                    self.state.progress.remove(&key);
                    self.save_state();
                    self.toast("Progress reset.");
                }
            }
            Confirm::DeleteMine(index) => {
                let key = self.lib.books[index].path.clone();
                match self.lib.remove_mine(index) {
                    Ok(()) => {
                        log::info!("deleted text {key:?}");
                        self.state.progress.remove(&key);
                        self.save_state();
                        self.refilter();
                    }
                    Err(e) => self.toast(format!("Could not delete: {e}")),
                }
            }
        }
    }

    /// List movement shared by every screen; returns the delta, if the key was one.
    fn nav_delta(key: KeyEvent, ctrl: bool) -> Option<isize> {
        Some(match key.code {
            KeyCode::Char('j') | KeyCode::Down => 1,
            KeyCode::Char('k') | KeyCode::Up => -1,
            KeyCode::Char('d') if ctrl => 12,
            KeyCode::Char('u') if ctrl => -12,
            KeyCode::Char('n') if ctrl => 1,
            KeyCode::Char('p') if ctrl => -1,
            KeyCode::PageDown => 20,
            KeyCode::PageUp => -20,
            KeyCode::Char('g') | KeyCode::Home => isize::MIN / 2,
            KeyCode::Char('G') | KeyCode::End => isize::MAX / 2,
            _ => return None,
        })
    }

    fn key_library(&mut self, key: KeyEvent, ctrl: bool) {
        if let Some(d) = Self::nav_delta(key, ctrl) {
            self.lib_sel = step(self.filtered.len(), self.lib_sel, d);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Esc if !self.query.is_empty() => {
                self.query.clear();
                self.refilter();
            }
            KeyCode::Tab | KeyCode::Char(']') => {
                self.tab = cycle(&Tab::ALL, self.tab, 1);
                self.refilter();
            }
            KeyCode::BackTab | KeyCode::Char('[') => {
                self.tab = cycle(&Tab::ALL, self.tab, -1);
                self.refilter();
            }
            KeyCode::Char('s') => {
                self.sort = cycle(&Sort::ALL, self.sort, 1);
                self.refilter();
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(i) = self.selected_book() {
                    self.open_book(i);
                }
            }
            KeyCode::Char('c') => {
                let recent = (0..self.lib.books.len())
                    .filter(|&i| self.progress_of(i).is_some_and(|p| !p.finished()))
                    .max_by_key(|&i| self.progress_of(i).map(|p| p.last_at));
                match recent {
                    Some(i) => self.open_book(i),
                    None => self.toast("Nothing in progress yet - pick a book and press Enter."),
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(i) = self.selected_book().filter(|&i| self.lib.books[i].mine) {
                    self.confirm = Some(Confirm::DeleteMine(i));
                }
            }
            _ => {}
        }
    }

    fn key_search(&mut self, key: KeyEvent, ctrl: bool) {
        match key.code {
            KeyCode::Esc => {
                self.searching = false;
                self.query.clear();
                self.refilter();
            }
            KeyCode::Enter => self.searching = false,
            KeyCode::Down => self.lib_sel = step(self.filtered.len(), self.lib_sel, 1),
            KeyCode::Up => self.lib_sel = step(self.filtered.len(), self.lib_sel, -1),
            KeyCode::Char('n') if ctrl => self.lib_sel = step(self.filtered.len(), self.lib_sel, 1),
            KeyCode::Char('p') if ctrl => self.lib_sel = step(self.filtered.len(), self.lib_sel, -1),
            KeyCode::Char('u') | KeyCode::Char('w') if ctrl => {
                self.query.clear();
                self.refilter();
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.refilter();
            }
            _ => {}
        }
    }

    fn key_book(&mut self, key: KeyEvent, ctrl: bool) {
        let Some(open) = &mut self.open else {
            self.screen = Screen::Library;
            return;
        };
        if let Some(d) = Self::nav_delta(key, ctrl) {
            open.sel = step(open.book.chapters.len(), open.sel, d);
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
                self.screen = Screen::Library;
                self.refilter_keep_selection();
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                let ch = open.sel;
                self.start_typing(ch, None);
            }
            KeyCode::Char('c') => {
                let ch = self.resume_chapter();
                if let Some(o) = &mut self.open {
                    o.sel = ch;
                }
                self.start_typing(ch, None);
            }
            KeyCode::Char('r') if self.book_key().is_some_and(|k| self.state.progress.contains_key(&k)) => {
                self.confirm = Some(Confirm::ResetBook);
            }
            _ => {}
        }
    }

    /// Re-applies the filter (progress may have changed) without losing the cursor.
    fn refilter_keep_selection(&mut self) {
        let current = self.selected_book();
        self.refilter();
        if let Some(pos) = current.and_then(|c| self.filtered.iter().position(|&i| i == c)) {
            self.lib_sel = pos;
        }
    }

    fn key_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.settings_sel = step(SETTING_LABELS.len(), self.settings_sel, 1),
            KeyCode::Char('k') | KeyCode::Up => self.settings_sel = step(SETTING_LABELS.len(), self.settings_sel, -1),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => self.change_setting(self.settings_sel, 1),
            KeyCode::Char('h') | KeyCode::Left => self.change_setting(self.settings_sel, -1),
            KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Library,
            _ => {}
        }
    }

    fn key_typing(&mut self, key: KeyEvent, ctrl: bool) {
        let now = self.now_ms();
        let Some(t) = &mut self.typing else {
            self.screen = Screen::Book;
            return;
        };
        if t.result.is_some() {
            match key.code {
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('n') | KeyCode::Right => self.next_page(),
                KeyCode::Char('r') => {
                    let page = t.page;
                    self.goto_page(page)
                }
                KeyCode::Char('p') | KeyCode::Left => self.prev_page(),
                KeyCode::Esc | KeyCode::Char('q') => self.leave_typing(),
                _ => {}
            }
            return;
        }
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => self.leave_typing(),
            KeyCode::Char('r') if ctrl => {
                let page = t.page;
                self.goto_page(page)
            }
            KeyCode::Char('n') if ctrl => {
                self.save_partial();
                self.next_page()
            }
            KeyCode::Char('p') if ctrl => {
                self.save_partial();
                self.prev_page()
            }
            KeyCode::Char('w') | KeyCode::Char('h') if ctrl => t.session.backspace_word(),
            KeyCode::Backspace if ctrl || alt => t.session.backspace_word(),
            KeyCode::Backspace => t.session.backspace(),
            KeyCode::Enter => t.session.type_char('\n', now),
            KeyCode::Char(c) if !ctrl && !alt => t.session.type_char(c, now),
            _ => {}
        }
        if self.typing.as_ref().is_some_and(|t| t.result.is_none() && t.session.done()) {
            self.complete_page();
        }
    }

    fn leave_typing(&mut self) {
        self.save_partial();
        if let (Some(t), Some(o)) = (&self.typing, &mut self.open) {
            o.sel = t.ch;
        }
        self.typing = None;
        self.screen = Screen::Book;
    }
}
