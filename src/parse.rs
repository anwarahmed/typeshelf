//! Markdown book -> chapters of typeable lines.
//!
//! Books in classic-books-markdown share a header (`# Title:`, `## Author:`, `## Year:`,
//! then a dashed rule) but use heading levels inconsistently, so any heading starts a
//! section and headings without body text become group labels ("ACT I", "BOOK II").

use crate::normalize::normalize_text;

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// Starts a new paragraph (rendered with a gap above).
    pub para: bool,
}

#[derive(Clone, Debug)]
pub struct Chapter {
    pub title: String,
    /// Enclosing part/act/book labels, e.g. "ACT I". Empty when there are none.
    pub group: String,
    pub lines: Vec<Line>,
    /// Size of the chapter in typed characters (each line costs its length + 1).
    pub length: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Book {
    pub title: String,
    pub author: String,
    pub year: String,
    pub chapters: Vec<Chapter>,
}

/// A lone chapter longer than this is split into parts so it stays navigable.
const PART_SIZE: u32 = 30_000;
/// Hard-wrapped prose has lines at least this long; verse is shorter.
const WRAP_MEDIAN: usize = 58;

pub fn chapter_length(lines: &[Line]) -> u32 {
    lines.iter().map(|l| l.text.chars().count() as u32 + 1).sum()
}

/// Strips markdown emphasis, escapes and bullets, then normalizes the text.
fn clean_inline(raw: &str) -> String {
    let raw = raw.replace("**", "").replace("__", "");
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        if c == '\\' && next.is_some_and(|n| n.is_ascii_punctuation()) {
            out.push(next.unwrap());
            i += 2;
            continue;
        }
        if c == '*' || c == '_' {
            let open = prev.is_none_or(|p| !p.is_alphanumeric()) && next.is_some_and(|n| !n.is_whitespace());
            let close = prev.is_some_and(|p| !p.is_whitespace()) && next.is_none_or(|n| !n.is_alphanumeric());
            if open || close {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    let text = normalize_text(&out);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.strip_prefix("* ").map(str::to_string).unwrap_or(text)
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &line[level..];
    rest.starts_with([' ', '\t']).then(|| (level, rest.trim()))
}

fn is_rule(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && ["-", "*", "_"].iter().any(|m| t.chars().all(|c| m.starts_with(c)))
}

struct Section {
    title: String,
    group: String,
    lines: Vec<Line>,
}

struct Ctx {
    level: usize,
    title: String,
    label: bool,
}

struct Builder {
    sections: Vec<Section>,
    /// Enclosing headings, plus same-level headings that had no body.
    ctx: Vec<Ctx>,
    cur: Section,
    in_heading: bool,
    block: Vec<(String, bool)>,
}

impl Builder {
    fn flush_block(&mut self) {
        if self.block.is_empty() {
            return;
        }
        let block = std::mem::take(&mut self.block);
        let hard = block.iter().any(|b| b.1);
        let join = !hard && block.len() > 1 && {
            let mut lens: Vec<usize> = block[..block.len() - 1].iter().map(|b| b.0.chars().count()).collect();
            lens.sort_unstable();
            lens[lens.len() / 2] >= WRAP_MEDIAN
        };
        if join {
            let text = block.iter().map(|b| b.0.as_str()).collect::<Vec<_>>().join(" ");
            self.cur.lines.push(Line { text, para: true });
        } else {
            for (i, (text, _)) in block.into_iter().enumerate() {
                self.cur.lines.push(Line { text, para: i == 0 });
            }
        }
    }

    fn close_section(&mut self) {
        self.flush_block();
        let cur = std::mem::replace(&mut self.cur, Section { title: String::new(), group: String::new(), lines: vec![] });
        if !cur.lines.is_empty() {
            self.sections.push(cur);
        } else if self.in_heading {
            // A heading with nothing under it labels what follows, replacing older
            // labels of its level.
            let last = self.ctx.len() - 1;
            self.ctx[last].label = true;
            let level = self.ctx[last].level;
            let mut i = 0;
            self.ctx.retain(|c| {
                i += 1;
                i - 1 == last || !(c.level == level && c.label)
            });
        }
    }

    fn open_heading(&mut self, level: usize, text: &str) {
        self.close_section();
        let title = match clean_inline(text) {
            t if t.is_empty() => "Untitled".to_string(),
            t => t,
        };
        self.ctx.retain(|c| c.level < level || (c.level == level && c.label));
        let group = self.ctx.iter().map(|c| c.title.as_str()).collect::<Vec<_>>().join(" · ");
        self.cur = Section { title: title.clone(), group, lines: vec![] };
        self.ctx.push(Ctx { level, title, label: false });
        self.in_heading = true;
    }
}

/// Reads the `# Title:` / `## Author:` / `## Year:` header, returning the body offset.
fn parse_header(src: &[&str], book: &mut Book) -> usize {
    let Some(rule) = src.iter().take(15).position(|l| l.len() >= 5 && l.trim_end().bytes().all(|b| b == b'-')) else {
        return 0;
    };
    let first = src.iter().find(|l| !l.trim().is_empty()).copied().unwrap_or("");
    if !first.trim_start_matches('#').trim_start().to_lowercase().starts_with("title:") || !first.starts_with('#') {
        return 0;
    }
    for l in &src[..rule] {
        let Some((_, rest)) = heading(l) else { continue };
        let Some((key, value)) = rest.split_once(':') else { continue };
        let value = value.trim().to_string();
        match key.to_lowercase().as_str() {
            "title" => book.title = value,
            "author" => book.author = value,
            "year" => book.year = value,
            _ => {}
        }
    }
    rule + 1
}

pub fn parse_book(md: &str, fallback_title: &str) -> Book {
    let md = md.replace("\r\n", "\n").replace('\r', "\n");
    let src: Vec<&str> = md.trim_start_matches('\u{FEFF}').split('\n').collect();
    let mut book = Book { title: fallback_title.to_string(), ..Default::default() };
    let body = parse_header(&src, &mut book);

    let mut b =
        Builder { sections: vec![], ctx: vec![], cur: Section { title: String::new(), group: String::new(), lines: vec![] }, in_heading: false, block: vec![] };
    for raw in &src[body..] {
        if let Some((level, text)) = heading(raw) {
            b.open_heading(level, text);
            continue;
        }
        if raw.trim().is_empty() || is_rule(raw) {
            b.flush_block();
            continue;
        }
        let quoted = raw.trim_start().trim_start_matches(['>', ' ']);
        if quoted.starts_with("![") {
            continue;
        }
        let text = clean_inline(quoted);
        if !text.is_empty() {
            b.block.push((text, raw.ends_with("  ")));
        }
    }
    b.close_section();

    let many = b.sections.len() > 1;
    let mut chapters: Vec<Chapter> = b
        .sections
        .into_iter()
        .enumerate()
        .map(|(i, s)| Chapter {
            title: match s.title {
                t if !t.is_empty() => t,
                _ if many && i == 0 => "Opening".to_string(),
                _ => book.title.clone(),
            },
            group: s.group,
            length: chapter_length(&s.lines),
            lines: s.lines,
        })
        .collect();

    if chapters.len() == 1 && chapters[0].length > PART_SIZE * 3 / 2 {
        chapters = split_into_parts(chapters.remove(0));
    }
    book.chapters = chapters;
    book
}

fn split_into_parts(ch: Chapter) -> Vec<Chapter> {
    let mut parts: Vec<Vec<Line>> = vec![vec![]];
    let mut size = 0;
    for line in ch.lines {
        if size >= PART_SIZE && line.para {
            parts.push(vec![]);
            size = 0;
        }
        size += line.text.chars().count() as u32 + 1;
        parts.last_mut().unwrap().push(line);
    }
    parts
        .into_iter()
        .enumerate()
        .map(|(i, lines)| Chapter { title: format!("Part {}", i + 1), group: String::new(), length: chapter_length(&lines), lines })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "# Title: T\r\n\r\n## Author: A\r\n\r\n## Year: 1900\r\n\r\n-------\r\n\r\n";

    #[test]
    fn reads_header_and_chapters() {
        let b = parse_book(&format!("{HEADER}## One\n\nHello _there_.\n\nBye.\n\n## Two\n\nMore **text**.\n"), "x");
        assert_eq!((b.title.as_str(), b.author.as_str(), b.year.as_str()), ("T", "A", "1900"));
        assert_eq!(b.chapters.len(), 2);
        assert_eq!(b.chapters[0].title, "One");
        assert_eq!(b.chapters[0].lines[0].text, "Hello there.");
        assert_eq!(b.chapters[0].length, 18);
        assert_eq!(b.chapters[1].lines[0].text, "More text.");
    }

    #[test]
    fn empty_headings_become_groups() {
        let b = parse_book(&format!("{HEADER}## ACT I\n\n### SCENE I\n\nA\n\n### SCENE II\n\nB\n\n## ACT II\n\n### SCENE I\n\nC\n"), "x");
        let got: Vec<_> = b.chapters.iter().map(|c| (c.group.as_str(), c.title.as_str())).collect();
        assert_eq!(got, [("ACT I", "SCENE I"), ("ACT I", "SCENE II"), ("ACT II", "SCENE I")]);
    }

    #[test]
    fn same_level_labels() {
        let b = parse_book(&format!("{HEADER}## BOOK I\n\n## CH 1\n\nA\n\n## BOOK II\n\n## CH 1\n\nB\n"), "x");
        let got: Vec<_> = b.chapters.iter().map(|c| c.group.as_str()).collect();
        assert_eq!(got, ["BOOK I", "BOOK II"]);
    }

    #[test]
    fn verse_keeps_lines_and_prose_joins() {
        let b = parse_book("Piping down the valleys wild,\nPiping songs of pleasant glee,\n\nx", "Poem");
        assert_eq!(b.chapters[0].lines.len(), 3);
        assert!(!b.chapters[0].lines[1].para);
        let long = "word ".repeat(13);
        let b = parse_book(&format!("{long}\n{long}\nend."), "Prose");
        assert_eq!(b.chapters[0].lines.len(), 1);
        assert_eq!(b.title, "Prose");
    }

    #[test]
    fn keeps_non_emphasis_markers() {
        assert_eq!(clean_inline("snake_case and 2 * 3"), "snake_case and 2 * 3");
        assert_eq!(clean_inline("_Enter HAMLET_  "), "Enter HAMLET");
        assert_eq!(clean_inline("_spans a line"), "spans a line");
    }
}
