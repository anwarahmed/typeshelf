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

use std::io::stdout;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

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
  typeshelf update          check for a newer version now and install it
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
            println!("typeshelf {} ({})", env!("CARGO_PKG_VERSION"), if update::COMMIT.is_empty() { "unknown commit" } else { update::COMMIT });
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

fn run(app: &mut App) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let mut cursor = String::new();
    let result = loop {
        app.tick();
        if cursor != app.settings.cursor {
            cursor = app.settings.cursor.clone();
            execute!(stdout(), cursor_style(&cursor))?;
        }
        if let Err(e) = terminal.draw(|f| ui::draw(f, app)) {
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
