mod app;
mod engine;
mod library;
mod log;
mod normalize;
mod paginate;
mod parse;
mod store;
mod theme;
mod ui;
mod update;

use std::cell::RefCell;
use std::io::{self, Write, stdout};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::execute;

use app::App;
use library::Library;
use store::{Settings, State};

const USAGE: &str = "\
typeshelf - practice typing by retyping classic books

Usage:
  typeshelf                 open the library
  typeshelf <file>          add a text or markdown file to your texts and open it
  typeshelf sync            download every book for offline use
  typeshelf update          check for a newer release now and install it
  typeshelf --help | --version

Environment:
  TYPESHELF_BOOKS           path to a local clone of classic-books-markdown
  TYPESHELF_NO_UPDATE       set to skip the update check at startup
  TYPESHELF_LOG             where to write the log (default ~/.local/state/typeshelf/typeshelf.log)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let settings = Settings::load();
    let books_dir = std::env::var_os("TYPESHELF_BOOKS").map(PathBuf::from).or_else(|| settings.books_dir.as_ref().map(PathBuf::from));
    let mut lib = Library::load(books_dir);

    let mut open = None;
    match args.first().map(String::as_str) {
        None => {}
        Some("-h" | "--help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version") => {
            println!("typeshelf {} ({})", update::VERSION, if update::COMMIT.is_empty() { "unknown commit" } else { update::COMMIT });
            return ExitCode::SUCCESS;
        }
        Some("sync") => return sync(&lib),
        Some("update") => {
            log::init();
            return match update::command(&settings) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            };
        }
        // Maintainer command: regenerate the embedded catalog from a clone of the books repo.
        Some("build-catalog") => {
            let Some(repo) = args.get(1) else {
                eprintln!("usage: typeshelf build-catalog <path-to-classic-books-markdown> [out.json]");
                return ExitCode::FAILURE;
            };
            let out = args.get(2).map_or("assets/catalog.json", String::as_str);
            return match library::build_catalog(Path::new(repo), Path::new(out)) {
                Ok(n) => {
                    println!("wrote {n} books to {out}");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            };
        }
        // Maintainer command: print how a file parses, to check the parser against a book.
        Some("dump") => {
            let Some(md) = args.get(1).and_then(|f| std::fs::read_to_string(f).ok()) else { return fail("usage: typeshelf dump <file>") };
            let book = parse::parse_book(&md, "Untitled");
            println!("{} | {} | {}", book.title, book.author, book.year);
            for (i, ch) in book.chapters.iter().enumerate() {
                println!("\n=== {} [{}] {} ({} chars, {} pages)", i + 1, ch.group, ch.title, ch.length, paginate::paginate(&ch.lines, 800).len());
                for l in &ch.lines {
                    println!("{}{}", if l.para { "\n" } else { "" }, l.text);
                }
            }
            return ExitCode::SUCCESS;
        }
        Some(flag) if flag.starts_with('-') => {
            eprint!("unknown option {flag}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
        Some(file) => match lib.import(Path::new(file)) {
            Ok(index) => open = Some(index),
            Err(e) => return fail(&e),
        },
    }

    log::init();
    log::info!(
        "start v{} ({}) on {}; TERM={:?} COLORTERM={:?}; {} books ({} own); theme {:?}, page {:?}, width {}",
        env!("CARGO_PKG_VERSION"),
        update::COMMIT,
        std::env::consts::OS,
        std::env::var("TERM").unwrap_or_default(),
        std::env::var("COLORTERM").unwrap_or_default(),
        lib.books.len(),
        lib.books.iter().filter(|b| b.mine).count(),
        settings.theme,
        settings.page_length,
        settings.text_width
    );
    update::before_start(&settings);

    // Installed before ratatui's hook, which restores the terminal and then calls this one.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
        default_hook(info)
    }));

    let mut app = App::new(settings, State::load(), lib);
    if let Some(index) = open {
        app.open_book(index);
    }
    match run(&mut app) {
        Ok(()) => {
            log::info!("exit");
            ExitCode::SUCCESS
        }
        Err(e) => {
            log::error!("terminal error: {e}");
            fail(&e.to_string())
        }
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("typeshelf: {msg}");
    ExitCode::FAILURE
}

fn cursor_style(name: &str) -> SetCursorStyle {
    match name {
        "block" => SetCursorStyle::SteadyBlock,
        "bar" => SetCursorStyle::SteadyBar,
        "underline" => SetCursorStyle::SteadyUnderScore,
        _ => SetCursorStyle::DefaultUserShape,
    }
}

/// Collects what ratatui writes during one frame, so `commit` can send it at once.
///
/// ratatui re-sends "show cursor" and "move cursor" on every frame, each flushed on its own, and
/// when it repaints the clock or the live stats it leaves the cursor there until a later write.
/// Terminals that paint between those writes (seen on macOS) show the cursor flickering.
/// Clones share one buffer: the backend owns one and `run` commits through the other.
#[derive(Clone, Default)]
struct FrameWriter(Rc<RefCell<Frames>>);

#[derive(Default)]
struct Frames {
    frame: Vec<u8>,
    last: Vec<u8>,
}

impl Write for FrameWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().frame.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl FrameWriter {
    /// Sends the frame in a single write, as a synchronized update where the terminal has them.
    /// A frame identical to the one before it changes nothing on screen (cells are only resent
    /// when they differ, and the cursor is positioned absolutely), so it is not sent at all:
    /// an idle screen writes nothing and the cursor keeps its blink.
    fn commit(&self, out: &mut impl Write) -> io::Result<()> {
        let Frames { frame, last } = &mut *self.0.borrow_mut();
        if frame != last {
            out.write_all(b"\x1b[?2026h")?;
            out.write_all(frame)?;
            out.write_all(b"\x1b[?2026l")?;
            out.flush()?;
        }
        std::mem::swap(frame, last);
        frame.clear();
        Ok(())
    }
}

fn run(app: &mut App) -> io::Result<()> {
    // Raw mode, alternate screen and the panic hook; drawing goes through `FrameWriter` instead.
    drop(ratatui::init());
    let frames = FrameWriter::default();
    let mut terminal = Terminal::new(CrosstermBackend::new(frames.clone()))?;
    let mut cursor = String::new();
    let result = loop {
        app.tick();
        if cursor != app.settings.cursor {
            cursor = app.settings.cursor.clone();
            execute!(stdout(), cursor_style(&cursor))?;
        }
        if let Err(e) = terminal.draw(|f| ui::draw(f, app)).and_then(|_| frames.commit(&mut stdout().lock())) {
            break Err(e);
        }
        // The timeout keeps the clock, live stats and download status moving.
        match event::poll(Duration::from_millis(200)) {
            Ok(true) => loop {
                match event::read() {
                    Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => app.on_key(key),
                    Ok(_) => {}
                    Err(e) => {
                        ratatui::restore();
                        return Err(e);
                    }
                }
                // Drain queued keys before redrawing so fast typing never lags.
                if !event::poll(Duration::ZERO).unwrap_or(false) {
                    break;
                }
            },
            Ok(false) => {}
            Err(e) => break Err(e),
        }
        if app.quit {
            break Ok(());
        }
    };
    let _ = execute!(stdout(), SetCursorStyle::DefaultUserShape);
    ratatui::restore();
    result
}

fn sync(lib: &Library) -> ExitCode {
    let todo: Vec<_> = lib.books.iter().filter(|b| !b.mine && !lib.is_local(b)).collect();
    println!("{} of {} books to download", todo.len(), lib.books.iter().filter(|b| !b.mine).count());
    let mut failed = 0;
    for (i, b) in todo.iter().enumerate() {
        match Library::download(&lib.url(b), &lib.cache_path(b)) {
            Ok(_) => println!("[{}/{}] {}", i + 1, todo.len(), b.title),
            Err(e) => {
                failed += 1;
                eprintln!("[{}/{}] {} - failed: {e}", i + 1, todo.len(), b.title);
            }
        }
    }
    if failed > 0 {
        eprintln!("{failed} downloads failed; run `typeshelf sync` again to retry them");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: &mut FrameWriter, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        w.write_all(bytes).unwrap();
        w.flush().unwrap();
        w.commit(&mut out).unwrap();
        out
    }

    #[test]
    fn frame_writer_sends_only_frames_that_differ() {
        let mut w = FrameWriter::default();
        assert_eq!(frame(&mut w, b"a"), b"\x1b[?2026ha\x1b[?2026l");
        assert!(frame(&mut w, b"a").is_empty());
        assert!(frame(&mut w, b"a").is_empty());
        assert_eq!(frame(&mut w, b"b"), b"\x1b[?2026hb\x1b[?2026l");
        assert_eq!(frame(&mut w, b"a"), b"\x1b[?2026ha\x1b[?2026l");
    }
}
