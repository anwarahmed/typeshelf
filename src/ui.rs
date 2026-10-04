//! Drawing. Every function here is a pure view of `App`.

use chrono::{DateTime, Datelike, Days, Local, NaiveDate};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Bar, BarChart, BarGroup, Block, BorderType, Chart, Clear, Dataset, GraphType, Padding, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Confirm, SETTING_LABELS, Screen, Tab, Typing};
use crate::engine::Mark;
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
fn window(sel: usize, len: usize, height: usize) -> usize {
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

// ---- frame ----

pub fn draw(f: &mut Frame, app: &App) {
    let st = styles(&app.theme);
    let area = f.area();
    f.render_widget(Block::new().style(st.base), area);
    let [top, body, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area.inner(ratatui::layout::Margin::new(1, 0)));

    draw_top(f, app, top, &st);
    let hints = match app.screen {
        Screen::Library => draw_library(f, app, body, &st),
        Screen::Book => draw_book(f, app, body, &st),
        Screen::Typing => draw_typing(f, app, body, &st),
        Screen::Stats => draw_stats(f, app, body, &st),
        Screen::Settings => draw_settings(f, app, body, &st),
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
            _ => 0,
        };
        for (i, name) in ["Library", "Stats", "Settings"].iter().enumerate() {
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

fn draw_typing<'a>(f: &mut Frame, app: &App, area: Rect, st: &Styles) -> Vec<(&'a str, &'a str)> {
    let Some(t) = &app.typing else { return vec![] };
    let s = &t.session;
    let [_, text_area, _, status] = Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1), Constraint::Length(2)]).areas(area);
    let width = app.settings.text_width.min(text_area.width.saturating_sub(2)).max(10).min(text_area.width);
    if width < 2 {
        return vec![];
    }
    let col = Rect::new(text_area.x + text_area.width.saturating_sub(width) / 2, text_area.y, width, text_area.height);

    // The last column is kept free for the space or line-break mark that ends a row.
    let rows = wrap_cells(t, width as usize - 1);
    let cursor_row = rows.iter().position(|r| r.contains(&s.pos)).unwrap_or(rows.len().saturating_sub(1));
    let h = col.height as usize;
    let first = cursor_row.saturating_sub(h / 3).min(rows.len().saturating_sub(h));

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
            if i == s.pos && t.result.is_none() {
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
        ])
    } else {
        Line::from(Span::styled(fmt_clock(live.ms), st.dim))
    };
    f.render_widget(Paragraph::new(info_line), info);
    f.render_widget(Paragraph::new(Span::styled(format!("{:.0}%", s.progress() * 100.0), st.dim)).alignment(Alignment::Right), info);

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
                Span::styled("esc", st.accent),
                Span::styled(" chapters", st.dim),
            ]),
        ];
        let r = popup(area, 52, 11);
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(panel(" page complete ", st)), r);
        return vec![];
    }
    vec![("esc", "chapters"), ("^r", "restart page"), ("^n/^p", "next/prev page"), ("^w", "delete word")]
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
    let sections: [(&str, &[(&str, &str)]); 4] = [
        (
            "Everywhere",
            &[
                ("1 2 3", "library, stats, settings"),
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
                ("ctrl-r", "restart page"),
                ("ctrl-n ctrl-p", "next, previous page"),
                ("esc", "back to chapters"),
            ],
        ),
    ];
    let mut lines = vec![];
    let current = match screen {
        Screen::Library => "Library",
        Screen::Book => "Book",
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
