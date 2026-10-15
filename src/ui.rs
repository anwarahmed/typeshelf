//! Drawing. Every function here is a pure view of `App`.

use chrono::{DateTime, Datelike, Days, Local, NaiveDate};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Bar, BarChart, BarGroup, Block, BorderType, Chart, Clear, Dataset, GraphType, Padding, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Confirm, DictState, SETTING_LABELS, Screen, Tab, Typing};
use crate::dict;
use crate::engine::{Cell, Mark};
use crate::store::{Record, cache_dir, config_dir, data_dir};
use crate::theme::Theme;

/// Assumed speed for time estimates until there is history to go on.
const DEFAULT_WPM: f32 = 40.0;

struct Styles {
    base: Style,
    dim: Style,
    accent: Style,
    bold: Style,
    sel: Style,
    error: Style,
    good: Style,
    fixed: Style,
}

fn styles(t: &Theme) -> Styles {
    let base = match t.bg {
        Some(bg) => Style::new().fg(t.fg).bg(bg),
        None => Style::new().fg(t.fg),
    };
    Styles {
        base,
        dim: base.fg(t.dim),
        accent: base.fg(t.accent),
        bold: base.add_modifier(Modifier::BOLD),
        sel: Style::new().fg(t.sel_fg).bg(t.sel_bg),
        error: base.fg(t.error),
        good: base.fg(t.good),
        fixed: base.fg(t.fixed),
    }
}

// ---- formatting ----

fn fmt_count(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..10_000 => format!("{:.1}k", n as f64 / 1000.0),
        10_000..1_000_000 => format!("{}k", n / 1000),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn fmt_clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}h {:02}m", s / 3600, s % 3600 / 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn fmt_hours(minutes: f32) -> String {
    if minutes < 60.0 { format!("{} min", minutes.round().max(1.0)) } else { format!("{:.1} h", minutes / 60.0) }
}

fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

fn pad(s: &str, width: usize) -> String {
    let t = truncate(s, width);
    let fill = width.saturating_sub(t.width());
    format!("{t}{}", " ".repeat(fill))
}

/// A thin progress bar, `width` cells wide.
fn meter<'a>(frac: f64, width: usize, st: &Styles) -> Vec<Span<'a>> {
    let filled = ((frac.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    let filled = if frac > 0.0 { filled.max(1) } else { 0 };
    vec![Span::styled("━".repeat(filled), st.accent), Span::styled("━".repeat(width - filled), st.dim)]
}

fn pct(frac: f64) -> String {
    if frac >= 1.0 {
        "done".into()
    } else if frac <= 0.0 {
        String::new()
    } else {
        format!("{:.0}%", (frac * 100.0).clamp(1.0, 99.0))
    }
}

/// First row to show so that `sel` sits mid-list.
pub fn window(sel: usize, len: usize, height: usize) -> usize {
    sel.saturating_sub(height / 2).min(len.saturating_sub(height))
}

fn popup(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn panel<'a>(title: &'a str, st: &Styles) -> Block<'a> {
    Block::bordered().border_type(BorderType::Rounded).border_style(st.dim).style(st.base).title(Span::styled(title, st.accent)).padding(Padding::horizontal(1))
}

fn avg_wpm(history: &[Record], last: usize) -> Option<f32> {
    let recent = &history[history.len().saturating_sub(last)..];
    (!recent.is_empty()).then(|| recent.iter().map(|r| r.wpm).sum::<f32>() / recent.len() as f32)
}

/// Greedy word wrap to `width` columns; only a word longer than a whole line is cut.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let (mut out, mut line, mut w) = (vec![], String::new(), 0);
    for word in text.split_whitespace() {
        if w > 0 && w + 1 + word.width() > width {
            out.push(std::mem::take(&mut line));
            w = 0;
        }
        if w > 0 {
            line.push(' ');
            w += 1;
        }
        for c in word.chars() {
            let cw = c.width().unwrap_or(0);
            if w > 0 && w + cw > width {
                out.push(std::mem::take(&mut line));
                w = 0;
            }
            line.push(c);
            w += cw;
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

// ---- frame ----

/// The top bar, the body and the footer.
pub fn frame_layout(area: Rect) -> [Rect; 3] {
    Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area.inner(Margin::new(1, 0)))
}

pub fn draw(f: &mut Frame, app: &App) {
    let st = styles(&app.theme);
    let area = f.area();
    f.render_widget(Block::new().style(st.base), area);
    let [top, body, bottom] = frame_layout(area);

    draw_top(f, app, top, &st);
    let hints = match app.screen {
        Screen::Library => draw_library(f, app, body, &st),
        Screen::Book => draw_book(f, app, body, &st),
        Screen::Typing => draw_typing(f, app, body, &st),
        Screen::Stats => draw_stats(f, app, body, &st),
        Screen::Settings => draw_settings(f, app, body, &st),
        Screen::Words => draw_words(f, app, body, &st),
    };
    draw_bottom(f, app, bottom, hints, &st);

    if let Some(i) = app.loading {
        let r = popup(area, 56, 5);
        f.render_widget(Clear, r);
        let text = vec![Line::from(truncate(&app.lib.books[i].title, 50)).style(st.bold), Line::from("Downloading…  esc to cancel").style(st.dim)];
        f.render_widget(Paragraph::new(text).alignment(Alignment::Center).block(panel("", &st).padding(Padding::uniform(1))), r);
    }
    if let Some(c) = &app.confirm {
        let msg = match c {
            Confirm::ResetBook => "Reset all progress in this book?".to_string(),
            Confirm::DeleteMine(i) => format!("Delete \"{}\" from your texts?", truncate(&app.lib.books[*i].title, 30)),
        };
        let r = popup(area, 56, 5);
        f.render_widget(Clear, r);
        let text = vec![
            Line::from(msg).style(st.bold),
            Line::from(vec![
                Span::styled("y", st.accent),
                Span::styled(" yes   ", st.dim),
                Span::styled("any other key", st.accent),
                Span::styled(" cancel", st.dim),
            ]),
        ];
        f.render_widget(Paragraph::new(text).alignment(Alignment::Center).block(panel("", &st).padding(Padding::uniform(1))), r);
    }
    if app.help {
        draw_help(f, app.screen, area, &st);
    }
}

fn draw_top(f: &mut Frame, app: &App, area: Rect, st: &Styles) {
    let mut spans = vec![Span::styled("typeshelf", st.accent.add_modifier(Modifier::BOLD)), Span::raw("   ")];
    if app.screen == Screen::Typing {
        if let (Some(o), Some(t)) = (&app.open, &app.typing) {
            let ch = &o.book.chapters[t.ch];
            spans.push(Span::styled(truncate(&o.book.title, 40), st.bold));
            let place = if ch.group.is_empty() { ch.title.clone() } else { format!("{} · {}", ch.group, ch.title) };
            spans.push(Span::styled(format!("  {}", truncate(&place, 50)), st.dim));
            let right = format!("page {} of {}", t.page + 1, t.pages.len());
            f.render_widget(Paragraph::new(Line::from(right).style(st.dim)).alignment(Alignment::Right), area);
        }
    } else {
        let current = match app.screen {
            Screen::Stats => 1,
            Screen::Settings => 2,
            Screen::Words => 3,
            _ => 0,
        };
        for (i, name) in ["Library", "Stats", "Settings", "Words"].iter().enumerate() {
            let style = if i == current { st.bold } else { st.dim };
            spans.push(Span::styled(format!("{} ", i + 1), st.dim));
            spans.push(Span::styled(format!("{name}   "), style));
        }
        let (level, frac) = app.state.level();
        let mut right = vec![Span::styled(format!("level {level}  "), st.dim)];
        right.extend(meter(frac, 10, st));
        f.render_widget(Paragraph::new(Line::from(right)).alignment(Alignment::Right), area);
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_bottom(f: &mut Frame, app: &App, area: Rect, hints: Vec<(&str, &str)>, st: &Styles) {
    if let Some((msg, _)) = &app.toast {
        f.render_widget(Paragraph::new(Span::styled(msg.as_str(), st.fixed)), area);
        return;
    }
    let mut spans = vec![];
    for (key, what) in hints {
        spans.push(Span::styled(key.to_string(), st.accent));
        spans.push(Span::styled(format!(" {what}   "), st.dim));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---- library ----

fn draw_library<'a>(f: &mut Frame, app: &App, area: Rect, st: &Styles) -> Vec<(&'a str, &'a str)> {
    let [tabs, search, _, body] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)]).areas(area);

    let mut spans = vec![];
    for tab in Tab::ALL {
        let style = if tab == app.tab { st.accent.add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { st.dim };
        spans.push(Span::styled(tab.label(), style));
        spans.push(Span::raw("   "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), tabs);
    let right = format!("{} {} · sorted by {}", app.filtered.len(), if app.filtered.len() == 1 { "book" } else { "books" }, app.sort.label());
    f.render_widget(Paragraph::new(Span::styled(right, st.dim)).alignment(Alignment::Right), tabs);

    let line = if app.searching {
        Line::from(vec![Span::styled("/ ", st.accent), Span::styled(app.query.as_str(), st.base)])
    } else if app.query.is_empty() {
        Line::from(Span::styled("/ search title or author", st.dim))
    } else {
        Line::from(vec![Span::styled("/ ", st.dim), Span::styled(app.query.as_str(), st.base), Span::styled("   esc clears", st.dim)])
    };
    f.render_widget(Paragraph::new(line), search);
    if app.searching {
        f.set_cursor_position(Position::new(search.x + 2 + app.query.width() as u16, search.y));
    }

    let wide = body.width >= 104;
    let [list, _, detail] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(if wide { 2 } else { 0 }), Constraint::Length(if wide { 38 } else { 0 })]).areas(body);

    if app.filtered.is_empty() {
        let msg = match app.tab {
            _ if !app.query.is_empty() => "No books match your search.",
            Tab::Reading => "Nothing in progress. Books you start typing show up here.",
            Tab::Finished => "No finished books yet.",
            Tab::Mine => "No texts of your own yet. Add one with:  typeshelf path/to/file.txt",
            Tab::All => "The catalog is empty.",
        };
        f.render_widget(Paragraph::new(Span::styled(msg, st.dim)), list);
    }

    let h = list.height as usize;
    let first = window(app.lib_sel, app.filtered.len(), h);
    let author_w = if list.width >= 70 { 22 } else { 0 };
    let title_w = (list.width as usize).saturating_sub(author_w + 6 + 8 + 14);
    for (row, &i) in app.filtered.iter().enumerate().skip(first).take(h) {
        let b = &app.lib.books[i];
        let frac = app.progress_of(i).map_or(0.0, |p| p.fraction());
        let selected = row == app.lib_sel;
        let (text, muted) = if selected { (st.sel, st.sel) } else { (st.base, st.dim) };
        let mut spans = vec![Span::styled(
            format!(" {} ", pad(&b.title, title_w.saturating_sub(2))),
            text.add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }),
        )];
        if author_w > 0 {
            spans.push(Span::styled(pad(&b.author, author_w), muted));
        }
        spans.push(Span::styled(format!("{:>5} ", truncate(&b.year, 5)), muted));
        spans.push(Span::styled(format!("{:>6}  ", fmt_count(b.chars as u64 / 5)), muted));
        if frac > 0.0 {
            spans.extend(meter(frac, 8, st).into_iter().map(|s| if selected { s.bg(app.theme.sel_bg) } else { s }));
            spans.push(Span::styled(format!(" {:<5}", pct(frac)), muted));
        } else {
            spans.push(Span::styled(" ".repeat(14), muted));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), Rect::new(list.x, list.y + (row - first) as u16, list.width, 1));
    }

    if wide && let Some(i) = app.selected_book() {
        draw_detail(f, app, i, detail, st);
    }

    if app.searching {
        vec![("enter", "keep filter"), ("esc", "clear"), ("↑↓", "move")]
    } else {
        vec![("↵", "open"), ("j/k", "move"), ("/", "search"), ("tab", "shelf"), ("s", "sort"), ("c", "continue"), ("?", "help"), ("q", "quit")]
    }
}

fn draw_detail(f: &mut Frame, app: &App, i: usize, area: Rect, st: &Styles) {
    let b = &app.lib.books[i];
    let words = b.chars as f32 / 5.0;
    let wpm = avg_wpm(&app.state.history, 20).unwrap_or(DEFAULT_WPM).max(5.0);
    let kv = |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<10}"), st.dim), Span::styled(v, st.base)]);
    let mut lines = vec![
        Line::from(Span::styled(b.title.clone(), st.bold)),
        Line::from(Span::styled(b.author.clone(), st.accent)),
        Line::raw(""),
        kv("Year", if b.year.is_empty() { "-".into() } else { b.year.clone() }),
        kv("Length", format!("{} words", fmt_count(words as u64))),
        kv("Chapters", b.chapters.to_string()),
        kv("Pages", format!("about {}", fmt_count((b.chars as usize / app.settings.page_target()).max(1) as u64))),
        kv("To type", format!("about {} at {:.0} wpm", fmt_hours(words / wpm), wpm)),
        kv(
            "Source",
            if b.mine {
                "your text"
            } else if app.lib.is_local(b) {
                "on disk"
            } else {
                "downloads on open"
            }
            .into(),
        ),
    ];
    if let Some(p) = app.progress_of(i) {
        lines.push(Line::raw(""));
        let mut m = meter(p.fraction(), 20, st);
        m.push(Span::styled(format!("  {}", pct(p.fraction())), st.base));
        lines.push(Line::from(m));
        let pages = app.state.history.iter().filter(|r| r.book == b.path).count();
        lines.push(Line::from(Span::styled(format!("{pages} pages typed"), st.dim)));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }).block(panel("", st).padding(Padding::new(2, 2, 1, 1))), area);
}

// ---- book ----

fn draw_book<'a>(f: &mut Frame, app: &App, area: Rect, st: &Styles) -> Vec<(&'a str, &'a str)> {
    let Some(open) = &app.open else { return vec![] };
    let meta = &app.lib.books[open.index];
    let progress = app.progress_of(open.index);
    let [head, _, list] =
        Layout::vertical([Constraint::Length(4), Constraint::Length(1), Constraint::Min(0)]).areas(area.inner(ratatui::layout::Margin::new(1, 1)));

    let frac = progress.map_or(0.0, |p| p.fraction());
    let mut bar = meter(frac, 30, st);
    bar.push(Span::styled(format!("  {}", if frac > 0.0 { pct(frac) } else { "not started".into() }), st.dim));
    if let Some((wpm, acc, pages)) = app.state.averages(&meta.path, None) {
        bar.push(Span::styled(format!("   ·   {wpm:.0} wpm   {acc:.1}% accuracy   {pages} {} typed", if pages == 1 { "page" } else { "pages" }), st.dim));
    }
    let byline = [meta.author.as_str(), meta.year.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
    let lines = vec![Line::from(Span::styled(open.book.title.clone(), st.bold)), Line::from(Span::styled(byline, st.accent)), Line::raw(""), Line::from(bar)];
    f.render_widget(Paragraph::new(lines), head);

    let chapters = &open.book.chapters;
    let h = list.height as usize;
    let first = window(open.sel, chapters.len(), h);
    let title_w = (list.width as usize).saturating_sub(5 + 12 + 16 + 9);
    for (row, ch) in chapters.iter().enumerate().skip(first).take(h) {
        let selected = row == open.sel;
        let (text, muted) = if selected { (st.sel.add_modifier(Modifier::BOLD), st.sel) } else { (st.base, st.dim) };
        let frac = progress.map_or(0.0, |p| p.chapter_fraction(row));
        let name = if ch.group.is_empty() { ch.title.clone() } else { format!("{} · {}", ch.group, ch.title) };
        let pages = open.pages[row];
        let mut spans = vec![
            Span::styled(format!(" {:>3}  ", row + 1), muted),
            Span::styled(pad(&name, title_w), if frac >= 1.0 && !selected { st.dim } else { text }),
            Span::styled(format!("{:>4} {:<6} ", pages, if pages == 1 { "page" } else { "pages" }), muted),
            Span::styled(app.state.averages(&meta.path, Some(row)).map_or(" ".repeat(9), |a| format!("{:>4.0} wpm ", a.0)), muted),
        ];
        if frac >= 1.0 {
            spans.push(Span::styled(format!("{:<15}", "✓ done"), if selected { st.sel } else { st.good }));
        } else if frac > 0.0 {
            spans.extend(meter(frac, 8, st).into_iter().map(|s| if selected { s.bg(app.theme.sel_bg) } else { s }));
            spans.push(Span::styled(format!(" {:<6}", pct(frac)), muted));
        } else {
            spans.push(Span::styled(" ".repeat(15), muted));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), Rect::new(list.x, list.y + (row - first) as u16, list.width, 1));
    }
    vec![("↵", "type chapter"), ("c", "continue"), ("j/k", "move"), ("r", "reset progress"), ("esc", "library"), ("?", "help")]
}

// ---- typing ----

/// Word-wraps the page into rows of cell indices; an empty row is a paragraph gap.
fn wrap_cells(t: &Typing, width: usize) -> Vec<Vec<usize>> {
    let cells = &t.session.cells;
    let w_of = |i: usize| if cells[i].ch == '\n' { 1 } else { cells[i].ch.width().unwrap_or(0) };
    let mut rows: Vec<Vec<usize>> = vec![];
    let mut start = 0;
    for (n, line) in t.pages[t.page].lines.iter().enumerate() {
        if n > 0 && line.para {
            rows.push(vec![]);
        }
        let end = start + line.text.chars().count();
        let mut row = vec![];
        let mut w = 0;
        let mut k = start;
        while k < end {
            let word = k;
            while k < end && cells[k].ch != ' ' {
                k += 1;
            }
            let word_w: usize = (word..k).map(w_of).sum();
            while k < end && cells[k].ch == ' ' {
                k += 1;
            }
            if w > 0 && w + word_w > width {
                rows.push(std::mem::take(&mut row));
                w = 0;
            }
            #[allow(clippy::needless_range_loop)]
            for i in word..k {
                // Only a word longer than the whole row is broken mid-word.
                if w + w_of(i) > width && cells[i].ch != ' ' {
                    rows.push(std::mem::take(&mut row));
                    w = 0;
                }
                row.push(i);
                w += w_of(i);
            }
        }
        if end < cells.len() {
            row.push(end);
        }
        rows.push(row);
        start = end + 1;
    }
    rows
}

fn cell_width(cell: &Cell) -> u16 {
    if cell.ch == '\n' { 1 } else { cell.ch.width().unwrap_or(0) as u16 }
}

/// Where the typing screen puts things. `App` uses it too, so that a click lands on the
/// word that is drawn there.
pub struct TypingView {
    /// The column of text.
    pub col: Rect,
    /// The page wrapped into rows of cell indices.
    pub rows: Vec<Vec<usize>>,
    /// The first row shown.
    pub first: usize,
    pub status: Rect,
    pub glossary: Option<Rect>,
}

/// Narrower than this, the glossary is left out.
const GLOSSARY_MIN: u16 = 28;

pub fn typing_view(app: &App, body: Rect) -> Option<TypingView> {
    let t = app.typing.as_ref()?;
    let [_, text_area, _, status] = Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1), Constraint::Length(2)]).areas(body);
    let width = app.settings.text_width.min(text_area.width.saturating_sub(2)).max(10).min(text_area.width);
    if width < 2 {
        return None;
    }
    // The glossary goes beside the text when there is room for both.
    let room = text_area.width.saturating_sub(width + 4).min(44);
    let glossary = (app.settings.glossary != "off" && room >= GLOSSARY_MIN).then_some(room);
    let x = text_area.x + text_area.width.saturating_sub(width + glossary.map_or(0, |g| g + 3)) / 2;
    let col = Rect::new(x, text_area.y, width, text_area.height);
    let glossary = glossary.map(|g| Rect::new(x + width + 3, text_area.y, g, text_area.height));

    // The last column is kept free for the space or line-break mark that ends a row.
    let rows = wrap_cells(t, width as usize - 1);
    // The text scrolls to keep the cursor, or the word being looked up, in view.
    let focus = t.lookup.as_ref().and_then(|l| t.words.get(l.word)).map_or(t.session.pos, |w| w.0);
    let focus_row = rows.iter().position(|r| r.contains(&focus)).unwrap_or(rows.len().saturating_sub(1));
    let h = col.height as usize;
    let first = focus_row.saturating_sub(h / 3).min(rows.len().saturating_sub(h));
    Some(TypingView { col, rows, first, status, glossary })
}

/// The row a cell is in, and its column within the row.
pub fn cell_xy(view: &TypingView, cells: &[Cell], i: usize) -> Option<(usize, u16)> {
    let row = view.rows.iter().position(|r| r.contains(&i))?;
    Some((row, view.rows[row].iter().take_while(|&&c| c != i).map(|&c| cell_width(&cells[c])).sum()))
}

/// The cell drawn at a screen position.
pub fn cell_at(view: &TypingView, cells: &[Cell], at: Position) -> Option<usize> {
    if !view.col.contains(at) {
        return None;
    }
    let row = view.rows.get(view.first + (at.y - view.col.y) as usize)?;
    let mut x = view.col.x;
    for &i in row {
        let w = cell_width(&cells[i]);
        if at.x >= x && at.x < x + w.max(1) {
            return Some(i);
        }
        x += w;
    }
    None
}

/// Where the open definition goes: under the word, or above it when there is more room there.
pub fn definition_rect(app: &App, view: &TypingView, body: Rect) -> Option<Rect> {
    let t = app.typing.as_ref()?;
    let l = t.lookup.as_ref().filter(|l| l.open)?;
    let (row, x) = cell_xy(view, &t.session.cells, t.words.get(l.word)?.0)?;
    let y = view.col.y + row.saturating_sub(view.first).min(view.col.height.saturating_sub(1) as usize) as u16;
    let w = 64.min(body.width);
    let want = 18.min(body.height);
    let (below, above) = (body.bottom().saturating_sub(y + 1), y.saturating_sub(body.y));
    let (top, h) = if below >= want || below >= above { (y + 1, want.min(below)) } else { (y - want.min(above), want.min(above)) };
    let left = (view.col.x + x).min(body.right().saturating_sub(w)).max(body.x);
    (h >= 3 && w >= 8).then(|| Rect::new(left, top, w, h))
}

/// The entries of the glossary's page list each take two rows: the word and its first sense.
fn glossary_first(app: &App, rows: u16) -> usize {
    let Some(t) = &app.typing else { return 0 };
    let sel = t.lookup.as_ref().and_then(|l| t.uncommon.iter().position(|&u| dict::key(&t.word(u)) == dict::key(&t.word(l.word))));
    window(sel.unwrap_or(0), t.uncommon.len(), (rows / 2) as usize)
}

/// The page-list entry of the glossary at a screen position, as an index into `Typing::words`.
pub fn glossary_hit(app: &App, view: &TypingView, at: Position) -> Option<usize> {
    let t = app.typing.as_ref()?;
    let inner = view.glossary?.inner(Margin::new(2, 1));
    if app.settings.glossary != "page" || !inner.contains(at) {
        return None;
    }
    t.uncommon.get(glossary_first(app, inner.height) + ((at.y - inner.y) / 2) as usize).copied()
}

/// How far the definition on screen can scroll, so the keys and the wheel stop at its end.
pub fn max_scroll(app: &App) -> usize {
    let st = styles(&app.theme);
    let body = frame_layout(app.size)[1];
    let (word, r) = match app.screen {
        Screen::Typing => {
            let Some(view) = typing_view(app, body) else { return 0 };
            let (Some(t), Some(r)) = (&app.typing, definition_rect(app, &view, body)) else { return 0 };
            (t.word(t.lookup.as_ref().map_or(0, |l| l.word)), r)
        }
        Screen::Words => match app.word_items().get(app.words_sel) {
            Some(w) => (w.clone(), words_layout(body)[2]),
            None => return 0,
        },
        _ => return 0,
    };
    definition_lines(app, &word, r.width.saturating_sub(4) as usize, false, &st).len().saturating_sub(r.height.saturating_sub(2) as usize)
}

/// What the dictionary says about `word`, wrapped to `width`. `brief` leaves out the
/// examples and other words, and keeps three senses of each part of speech.
fn definition_lines(app: &App, word: &str, width: usize, brief: bool, st: &Styles) -> Vec<Line<'static>> {
    let note = |s: String| wrap_words(&s, width).into_iter().map(|l| Line::from(Span::styled(l, st.dim))).collect();
    let d = match &app.dict {
        DictState::Ready(d) => d,
        DictState::Downloading => return note(format!("Downloading the dictionary ({}, only this once)…", dict::DOWNLOAD_SIZE)),
        DictState::Failed(e) => return note(format!("The dictionary could not be loaded: {e}. It is tried again at the next look-up.")),
        DictState::Unloaded => return note("Loading the dictionary…".into()),
    };
    let entries = d.define(word);
    if entries.is_empty() {
        return note(format!("\"{word}\" is not in the dictionary, which has nouns, verbs, adjectives and adverbs but not words like \"the\" or \"upon\"."));
    }
    let mut out = vec![];
    let body = width.saturating_sub(4);
    for (n, e) in entries.iter().enumerate() {
        if n > 0 {
            out.push(Line::raw(""));
        }
        // The dictionary form, when it isn't simply the word itself.
        if entries.len() > 1 || e.lemma.replace(' ', "_") != dict::key(word) {
            out.push(Line::from(Span::styled(e.lemma.clone(), st.bold)));
        }
        let (mut pos, mut num) = (None, 0);
        for s in &e.senses {
            if pos != Some(s.pos) {
                out.push(Line::from(Span::styled(s.pos.label(), st.dim.add_modifier(Modifier::ITALIC))));
                (pos, num) = (Some(s.pos), 0);
            }
            num += 1;
            if brief && num > 3 {
                continue;
            }
            for (i, l) in wrap_words(&s.def, body).into_iter().enumerate() {
                out.push(Line::from(vec![Span::styled(if i == 0 { format!("{num:>2}. ") } else { "    ".into() }, st.dim), Span::styled(l, st.base)]));
            }
            if brief {
                continue;
            }
            for ex in s.examples.iter().take(2) {
                out.extend(wrap_words(&format!("\"{ex}\""), body).into_iter().map(|l| Line::from(Span::styled(format!("    {l}"), st.dim))));
            }
            if !s.also.is_empty() {
                out.extend(wrap_words(&format!("also: {}", s.also.join(", ")), body).into_iter().map(|l| Line::from(Span::styled(format!("    {l}"), st.dim))));
            }
        }
    }
    out
}

/// A definition in a panel, scrolled, with the dictionary credited on the bottom border.
fn draw_definition(f: &mut Frame, app: &App, word: &str, scroll: usize, hints: &[(&str, &str)], r: Rect, st: &Styles) {
    let lines = definition_lines(app, word, r.width.saturating_sub(4) as usize, false, st);
    let h = r.height.saturating_sub(2) as usize;
    let scroll = scroll.min(lines.len().saturating_sub(h));
    let mut keys = vec![Span::raw(" ")];
    for (k, what) in hints {
        keys.push(Span::styled(k.to_string(), st.accent));
        keys.push(Span::styled(format!(" {what}  "), st.dim));
    }
    if lines.len() > scroll + h {
        keys.push(Span::styled("↓ more ", st.dim));
    }
    let title = format!(" {word} ");
    let mut block = panel(&title, st).title_bottom(Line::from(keys));
    if r.width >= 56 {
        block = block.title_bottom(Line::from(Span::styled(" Open English WordNet ", st.dim)).right_aligned());
    }
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines[scroll..].to_vec()).block(block), r);
}

fn draw_glossary(f: &mut Frame, app: &App, t: &Typing, r: Rect, st: &Styles) {
    let inner = r.inner(Margin::new(2, 1));
    let width = inner.width as usize;
    let lines = if app.settings.glossary == "word" {
        let at = t.session.pos;
        let word = t.lookup.as_ref().map(|l| l.word).or_else(|| t.words.iter().position(|w| w.1 > at)).map(|w| t.word(w)).unwrap_or_default();
        if word.is_empty() { vec![] } else { definition_lines(app, &word, width, true, st) }
    } else if app.dictionary().is_none() {
        definition_lines(app, "", width, true, st)
    } else if t.uncommon.is_empty() {
        vec![Line::from(Span::styled("No uncommon words on this page.", st.dim))]
    } else {
        let sel = t.lookup.as_ref().map(|l| dict::key(&t.word(l.word)));
        let d = app.dictionary();
        t.uncommon
            .iter()
            .skip(glossary_first(app, inner.height))
            .take((inner.height / 2) as usize)
            .flat_map(|&u| {
                let word = t.word(u);
                let style = if sel.as_deref() == Some(dict::key(&word).as_str()) { st.sel.add_modifier(Modifier::BOLD) } else { st.bold };
                let def = d.and_then(|d| d.define(&word).into_iter().next()).and_then(|e| e.senses.into_iter().next()).map(|s| s.def).unwrap_or_default();
                [
                    Line::from(Span::styled(truncate(&word.to_lowercase(), width), style)),
                    Line::from(Span::styled(format!("  {}", truncate(&def, width.saturating_sub(2))), st.dim)),
                ]
            })
            .collect()
    };
    let title = if app.settings.glossary == "word" { " glossary " } else { " uncommon words " };
    f.render_widget(Paragraph::new(lines).block(panel(title, st).padding(Padding::new(1, 1, 0, 0))), r);
}

fn draw_typing<'a>(f: &mut Frame, app: &App, area: Rect, st: &Styles) -> Vec<(&'a str, &'a str)> {
    let Some(t) = &app.typing else { return vec![] };
    let s = &t.session;
    let Some(view) = typing_view(app, area) else { return vec![] };
    let TypingView { col, rows, first, status, glossary } = &view;
    let (col, first, status) = (*col, *first, *status);
    let h = col.height as usize;
    let picked = t.lookup.as_ref().and_then(|l| t.words.get(l.word)).copied();

    for (y, row) in rows.iter().enumerate().skip(first).take(h) {
        let mut spans = Vec::with_capacity(row.len());
        let mut x = 0u16;
        for &i in row {
            let cell = &s.cells[i];
            let newline = cell.ch == '\n';
            let (glyph, style) = match s.marks[i] {
                Mark::Wrong => (
                    if newline {
                        '↵'
                    } else if cell.ch == ' ' {
                        '·'
                    } else {
                        cell.ch
                    },
                    st.error.add_modifier(Modifier::UNDERLINED),
                ),
                _ if newline => ('↵', st.dim),
                Mark::Pending => (cell.ch, st.dim),
                Mark::Correct if s.had_error[i] => (cell.ch, st.fixed),
                Mark::Correct | Mark::Skipped => (cell.ch, st.base),
            };
            let style = if picked.is_some_and(|(a, b)| a <= i && i < b) { st.sel.add_modifier(Modifier::BOLD) } else { style };
            if i == s.pos && t.result.is_none() && t.lookup.is_none() {
                f.set_cursor_position(Position::new(col.x + x, col.y + (y - first) as u16));
            }
            x += if newline { 1 } else { cell.ch.width().unwrap_or(0) as u16 };
            spans.push(Span::styled(glyph.to_string(), style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), Rect::new(col.x, col.y + (y - first) as u16, col.width, 1));
    }

    let [bar, info] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(Rect::new(col.x, status.y, col.width, 2));
    f.render_widget(Paragraph::new(Line::from(meter(s.progress(), bar.width as usize, st))), bar);
    let live = s.stats(app.now_ms());
    let paused = Span::styled(
        if t.lookup.is_some() && t.result.is_none() {
            "   clock stopped while you look up words"
        } else if s.paused(app.now_ms()) {
            "   paused"
        } else {
            ""
        },
        st.dim,
    );
    let info_line = if !s.started() {
        Line::from(Span::styled("start typing…", st.dim))
    } else if app.settings.live_stats {
        Line::from(vec![
            // The first couple of seconds give wild numbers.
            Span::styled(if live.ms >= 2000 { format!("{:.0}", live.wpm) } else { "–".into() }, st.bold),
            Span::styled(" wpm   ", st.dim),
            Span::styled(format!("{:.0}%", live.acc), st.bold),
            Span::styled(" accuracy   ", st.dim),
            Span::styled(fmt_clock(live.ms), st.bold),
            paused,
        ])
    } else {
        Line::from(vec![Span::styled(fmt_clock(live.ms), st.dim), paused])
    };
    f.render_widget(Paragraph::new(info_line), info);
    f.render_widget(Paragraph::new(Span::styled(format!("{:.0}%", s.progress() * 100.0), st.dim)).alignment(Alignment::Right), info);

    if let Some(g) = glossary {
        draw_glossary(f, app, t, *g, st);
    }
    if let Some(l) = &t.lookup {
        if let Some(r) = definition_rect(app, &view, area) {
            draw_definition(f, app, &t.word(l.word), l.scroll, &[("s", "save"), ("esc", "close")], r, st);
            return vec![("←→", "next word"), ("j/k", "scroll"), ("s", "save word"), ("esc", "close")];
        }
        return vec![("←→ ↑↓", "pick a word"), ("[ ]", "uncommon words"), ("↵", "define"), ("tab", "back to typing")];
    }
    if let Some(r) = &t.result {
        let avg = avg_wpm(&app.state.history[..app.state.history.len().saturating_sub(1)], 20);
        let compare = match avg {
            Some(a) if r.wpm >= a => Span::styled(format!("▲ {:.0} above your recent average", r.wpm - a), st.good),
            Some(a) => Span::styled(format!("▼ {:.0} below your recent average", a - r.wpm), st.dim),
            None => Span::styled("first page - this sets your baseline", st.dim),
        };
        let lines = vec![
            Line::raw(""),
            Line::from(vec![
                Span::styled(format!("{:.0}", r.wpm), st.accent.add_modifier(Modifier::BOLD)),
                Span::styled(" wpm      ", st.dim),
                Span::styled(format!("{:.1}%", r.acc), st.accent.add_modifier(Modifier::BOLD)),
                Span::styled(" accuracy", st.dim),
            ]),
            Line::from(compare),
            Line::raw(""),
            Line::from(Span::styled(
                format!("{} · {} characters · {} {}", fmt_clock(r.ms), r.chars, r.mistakes, if r.mistakes == 1 { "mistake" } else { "mistakes" }),
                st.dim,
            )),
            Line::raw(""),
            Line::from(vec![
                Span::styled("↵", st.accent),
                Span::styled(" next page   ", st.dim),
                Span::styled("r", st.accent),
                Span::styled(" retry   ", st.dim),
                Span::styled("l", st.accent),
                Span::styled(" look up   ", st.dim),
                Span::styled("esc", st.accent),
                Span::styled(" chapters", st.dim),
            ]),
        ];
        let r = popup(area, 52, 11);
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(panel(" page complete ", st)), r);
        return vec![];
    }
    vec![("esc", "chapters"), ("tab", "look up words"), ("^r", "restart page"), ("^n/^p", "next/prev page"), ("^w", "delete word")]
}

// ---- words ----

/// The search line, the list of words, and the definition of the selected one.
pub fn words_layout(body: Rect) -> [Rect; 3] {
    let [search, _, rest] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)]).areas(body.inner(Margin::new(1, 1)));
    if rest.width >= 72 {
        let [list, _, def] = Layout::horizontal([Constraint::Length(30), Constraint::Length(2), Constraint::Min(0)]).areas(rest);
        [search, list, def]
    } else {
        let [list, def] = Layout::vertical([Constraint::Percentage(40), Constraint::Min(0)]).areas(rest);
        [search, list, def]
    }
}

fn draw_words<'a>(f: &mut Frame, app: &App, area: Rect, st: &Styles) -> Vec<(&'a str, &'a str)> {
    let [search, list, def] = words_layout(area);
    let items = app.word_items();
    let q = app.words_query.as_str();
    let line = if app.words_searching {
        Line::from(vec![Span::styled("/ ", st.accent), Span::styled(q, st.base)])
    } else if q.is_empty() {
        Line::from(Span::styled("/ look up any word", st.dim))
    } else {
        Line::from(vec![Span::styled("/ ", st.dim), Span::styled(q, st.base), Span::styled("   esc clears", st.dim)])
    };
    f.render_widget(Paragraph::new(line), search);
    if app.words_searching {
        f.set_cursor_position(Position::new(search.x + 2 + q.width() as u16, search.y));
    }
    let n = items.len();
    let count =
        if q.is_empty() { format!("{n} saved {}", if n == 1 { "word" } else { "words" }) } else { format!("{n} {}", if n == 1 { "match" } else { "matches" }) };
    f.render_widget(Paragraph::new(Span::styled(count, st.dim)).alignment(Alignment::Right), search);

    if items.is_empty() {
        let msg = if !q.is_empty() && app.dictionary().is_some() {
            format!("No dictionary words start with \"{q}\".")
        } else if !q.is_empty() {
            String::new()
        } else {
            "No saved words yet. While typing, press tab to pick a word, enter to define it, and s to save it. Or press / to look up any word.".into()
        };
        f.render_widget(Paragraph::new(Span::styled(msg, st.dim)).wrap(Wrap { trim: true }), list);
    }
    let h = list.height as usize;
    let first = window(app.words_sel, n, h);
    let saved: Vec<_> = app.state.words.iter().rev().collect();
    for (row, word) in items.iter().enumerate().skip(first).take(h) {
        let selected = row == app.words_sel;
        let (text, muted) = if selected { (st.sel.add_modifier(Modifier::BOLD), st.sel) } else { (st.base, st.dim) };
        let w = list.width as usize;
        let mut spans = vec![Span::styled(format!(" {}", pad(word, w.saturating_sub(1).min(18))), text)];
        if q.is_empty()
            && let Some(sw) = saved.get(row)
        {
            spans.push(Span::styled(pad(&format!(" {}", sw.title), w.saturating_sub(19)), muted));
        } else {
            spans.push(Span::styled(" ".repeat(w.saturating_sub(19)), muted));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), Rect::new(list.x, list.y + (row - first) as u16, list.width, 1));
    }
    match items.get(app.words_sel) {
        Some(word) => draw_definition(f, app, word, app.words_scroll, &[("J/K", "scroll")], def, st),
        None => {
            let lines = if q.is_empty() { vec![] } else { definition_lines(app, q, def.width.saturating_sub(4) as usize, false, st) };
            f.render_widget(
                Paragraph::new(lines).block(panel("", st).title_bottom(Line::from(Span::styled(format!(" {} ", dict::CREDIT), st.dim)).right_aligned())),
                def,
            );
        }
    }
    if app.words_searching {
        vec![("enter", "keep"), ("esc", "clear"), ("↑↓", "move")]
    } else if !q.is_empty() {
        vec![("j/k", "move"), ("s", "save word"), ("J/K", "scroll"), ("/", "search"), ("esc", "clear"), ("?", "help")]
    } else {
        vec![("j/k", "move"), ("/", "look up a word"), ("J/K", "scroll"), ("d", "remove"), ("1", "library"), ("?", "help"), ("q", "back")]
    }
}

// ---- stats ----

fn local_day(at: u64) -> NaiveDate {
    DateTime::from_timestamp(at as i64, 0).unwrap_or_default().with_timezone(&Local).date_naive()
}

fn draw_stats<'a>(f: &mut Frame, app: &App, area: Rect, st: &Styles) -> Vec<(&'a str, &'a str)> {
    let hist = &app.state.history;
    let hints = vec![("1", "library"), ("3", "settings"), ("?", "help"), ("q", "back")];
    if hist.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("No pages typed yet. Finish a page and your numbers will show up here.", st.dim)).alignment(Alignment::Center),
            popup(area, area.width, 1),
        );
        return hints;
    }
    let [tiles, charts, lower] =
        Layout::vertical([Constraint::Length(4), Constraint::Fill(1), Constraint::Fill(1)]).areas(area.inner(ratatui::layout::Margin::new(0, 1)));

    let chars: u64 = hist.iter().map(|r| r.chars as u64).sum();
    let ms: u64 = hist.iter().map(|r| r.ms).sum();
    let keys: f64 = hist.iter().map(|r| r.chars as f64).sum();
    let acc = hist.iter().map(|r| r.acc as f64 * r.chars as f64).sum::<f64>() / keys.max(1.0);
    let best = hist.iter().map(|r| r.wpm).fold(0.0, f32::max);
    let cells: [(&str, String); 7] = [
        ("recent wpm", format!("{:.0}", avg_wpm(hist, 10).unwrap_or(0.0))),
        ("best wpm", format!("{best:.0}")),
        ("accuracy", format!("{acc:.1}%")),
        ("pages", hist.len().to_string()),
        ("words", fmt_count(chars / 5)),
        ("time typing", fmt_clock(ms)),
        ("level", app.state.level().0.to_string()),
    ];
    let cols = Layout::horizontal([Constraint::Ratio(1, 7); 7]).split(tiles);
    for (i, (label, value)) in cells.iter().enumerate() {
        let lines = vec![Line::from(Span::styled(value.clone(), st.bold)), Line::from(Span::styled(*label, st.dim))];
        f.render_widget(Paragraph::new(lines).block(Block::new().padding(Padding::new(1, 1, 1, 0))), cols[i]);
    }

    let [speed, daily] = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(charts);
    let recent = &hist[hist.len().saturating_sub(80)..];
    let pts: Vec<(f64, f64)> = recent.iter().enumerate().map(|(i, r)| (i as f64, r.wpm as f64)).collect();
    let lo = (pts.iter().map(|p| p.1).fold(f64::MAX, f64::min) / 10.0).floor() * 10.0;
    let hi = ((pts.iter().map(|p| p.1).fold(0.0, f64::max) / 10.0).floor() + 1.0) * 10.0;
    let title = format!(" wpm · last {} pages ", recent.len());
    let chart = Chart::new(vec![Dataset::default().marker(Marker::Braille).graph_type(GraphType::Line).style(st.accent).data(&pts)])
        .block(panel(&title, st))
        .style(st.base)
        .x_axis(Axis::default().style(st.dim).bounds([0.0, (pts.len().max(2) - 1) as f64]))
        .y_axis(Axis::default().style(st.dim).bounds([lo, hi]).labels([format!("{lo:.0}"), format!("{:.0}", (lo + hi) / 2.0), format!("{hi:.0}")]));
    f.render_widget(chart, speed);

    let today = Local::now().date_naive();
    let days = ((daily.width.saturating_sub(4)) / 4).clamp(1, 14) as u64;
    let bars: Vec<Bar> = (0..days)
        .rev()
        .map(|back| {
            let day = today.checked_sub_days(Days::new(back)).unwrap_or(today);
            let words: u64 = hist.iter().filter(|r| local_day(r.at) == day).map(|r| r.chars as u64 / 5).sum();
            Bar::default().value(words).label(format!("{:>2}", day.day())).text_value(if words > 0 { fmt_count(words) } else { String::new() })
        })
        .collect();
    let chart = BarChart::default()
        .block(panel(" words per day ", st))
        .style(st.base)
        .bar_width(3)
        .bar_gap(1)
        .bar_style(st.accent)
        .value_style(st.base.fg(app.theme.bg.unwrap_or(ratatui::style::Color::Black)).bg(app.theme.accent))
        .label_style(st.dim)
        .data(BarGroup::default().bars(&bars));
    f.render_widget(chart, daily);

    let [missed, sessions] = Layout::horizontal([Constraint::Percentage(34), Constraint::Percentage(66)]).areas(lower);
    let mut keys: Vec<(&String, &u32)> = app.state.missed.iter().collect();
    keys.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let top = keys.first().map_or(1, |k| *k.1).max(1) as f64;
    let bar_w = (missed.width as usize).saturating_sub(18).max(4);
    let lines: Vec<Line> = keys
        .iter()
        .take(missed.height.saturating_sub(2) as usize)
        .map(|(k, n)| {
            let name = match k.as_str() {
                " " => "space".to_string(),
                "\n" => "enter".to_string(),
                k => k.to_string(),
            };
            let filled = ((**n as f64 / top) * bar_w as f64).ceil() as usize;
            Line::from(vec![Span::styled(format!("{name:<6}"), st.bold), Span::styled("━".repeat(filled), st.accent), Span::styled(format!(" {n}"), st.dim)])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel(" most missed keys ", st)), missed);

    let title_w = (sessions.width as usize).saturating_sub(4 + 13 + 8 + 8 + 8);
    let lines: Vec<Line> = hist
        .iter()
        .rev()
        .take(sessions.height.saturating_sub(2) as usize)
        .map(|r| {
            let when = DateTime::from_timestamp(r.at as i64, 0).unwrap_or_default().with_timezone(&Local).format("%b %e %H:%M").to_string();
            Line::from(vec![
                Span::styled(format!("{when:<13}"), st.dim),
                Span::styled(pad(&r.title, title_w), st.base),
                Span::styled(format!("{:>4.0} wpm", r.wpm), st.bold),
                Span::styled(format!("{:>7.1}%", r.acc), st.base),
                Span::styled(format!("{:>8}", fmt_clock(r.ms)), st.dim),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel(" recent pages ", st)), sessions);
    hints
}

// ---- settings ----

fn draw_settings<'a>(f: &mut Frame, app: &App, area: Rect, st: &Styles) -> Vec<(&'a str, &'a str)> {
    let width = 64.min(area.width);
    let col = Rect::new(area.x + (area.width - width) / 2, area.y + 2.min(area.height), width, area.height.saturating_sub(2));
    let mut lines = vec![];
    for (i, label) in SETTING_LABELS.iter().enumerate() {
        let selected = i == app.settings_sel;
        let value = app.setting_value(i);
        let style = if selected { st.sel.add_modifier(Modifier::BOLD) } else { st.base };
        let arrows = if selected { st.sel } else { st.dim };
        let fill = (width as usize).saturating_sub(label.width() + value.width() + 8);
        lines.push(Line::from(vec![
            Span::styled(format!(" {label}{}", " ".repeat(fill)), style),
            Span::styled(" ‹ ", arrows),
            Span::styled(value, style),
            Span::styled(" › ", arrows),
        ]));
    }
    lines.push(Line::raw(""));
    let help = [
        "Themes other than \"terminal\" need a truecolor terminal.",
        "Each page is roughly this many characters: short 450, medium 800, long 1300, very long 2200.",
        "The width of the column of text you type.",
        "\"terminal\" keeps your terminal's own cursor shape.",
        "When on, the cursor waits until you type the right key.",
        "When on, Space is accepted at the end of a line as well as Enter.",
        "Show speed and accuracy while typing, not only afterwards.",
        "A panel beside the text: the definition of the word at the cursor, or the page's uncommon words. Needs a wide terminal; the dictionary (10 MB) downloads the first time.",
        "Check GitHub for a newer release each time typeshelf starts, and install it.",
    ];
    lines.push(Line::from(Span::styled(help[app.settings_sel], st.dim)));
    lines.push(Line::raw(""));
    lines.push(Line::raw(""));
    let books = app.lib.books_dir.as_ref().map_or("not set (books download on demand)".to_string(), |d| d.display().to_string());
    for (k, v) in [
        ("settings", config_dir().join("config.json").display().to_string()),
        ("history", data_dir().join("state.json").display().to_string()),
        ("book cache", cache_dir().join("books").display().to_string()),
        ("dictionary", dict::dir().display().to_string()),
        ("books dir", books),
    ] {
        lines.push(Line::from(vec![Span::styled(format!(" {k:<12}"), st.dim), Span::styled(v, st.dim)]));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), col);
    vec![("j/k", "move"), ("h/l", "change"), ("1", "library"), ("2", "stats"), ("q", "back")]
}

// ---- help ----

/// Keys for the current screen only, so the panel fits a 24-row terminal.
fn draw_help(f: &mut Frame, screen: Screen, area: Rect, st: &Styles) {
    let sections: [(&str, &[(&str, &str)]); 5] = [
        (
            "Everywhere",
            &[
                ("1 2 3 4", "library, stats, settings, words"),
                ("j k  ↓ ↑  wheel", "move"),
                ("g G", "first, last"),
                ("ctrl-d ctrl-u", "half page down, up"),
                ("ctrl-c", "quit"),
            ],
        ),
        (
            "Library",
            &[
                ("enter", "open book"),
                ("/", "search title or author"),
                ("tab  [ ]", "switch shelf"),
                ("s", "change sort order"),
                ("c", "continue the book you typed last"),
                ("d", "delete one of your own texts"),
            ],
        ),
        ("Book", &[("enter", "type the selected chapter"), ("c", "continue where you left off"), ("r", "reset progress"), ("esc", "back to library")]),
        (
            "Typing",
            &[
                ("enter", "line break (space works too)"),
                ("backspace", "fix a mistake"),
                ("ctrl-w  alt-backspace", "delete word"),
                ("ctrl-r  ctrl-n  ctrl-p", "restart, next, previous page"),
                ("tab  ctrl-d  click", "pick a word to look up, define one"),
                ("esc", "back to chapters"),
            ],
        ),
        ("Words", &[("/", "look up any word"), ("s", "save the word to your list"), ("d", "remove a saved word"), ("J K  wheel", "scroll the definition")]),
    ];
    let mut lines = vec![];
    let current = match screen {
        Screen::Library => "Library",
        Screen::Book => "Book",
        Screen::Words => "Words",
        _ => "",
    };
    for (name, keys) in sections.into_iter().filter(|s| ["Everywhere", "Typing", current].contains(&s.0)) {
        lines.push(Line::from(Span::styled(name, st.accent.add_modifier(Modifier::BOLD))));
        for (k, what) in keys {
            lines.push(Line::from(vec![Span::styled(format!("  {k:<24}"), st.bold), Span::styled(*what, st.dim)]));
        }
        lines.push(Line::raw(""));
    }
    lines.pop();
    let r = popup(area, 66, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(panel(" keys ", st)), r);
}
