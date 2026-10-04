//! Splits a chapter into pages of roughly `target` characters, preferring paragraph
//! boundaries and splitting over-long paragraphs at sentence ends.

use crate::parse::Line;

#[derive(Clone, Debug)]
pub struct Page {
    pub lines: Vec<Line>,
    /// Character offset within the chapter where the page ends; progress is measured
    /// against it, so it survives a change of page length.
    pub end: u32,
}

fn split_at(text: &[char], room: usize) -> (String, String) {
    let room = room.min(text.len() - 1).max(1);
    let window = &text[..=room];
    // Last sentence end in the window, else last space, else a hard cut.
    let mut cut = None;
    for i in 1..window.len() {
        if window[i] != ' ' {
            continue;
        }
        let mut j = i;
        while j > 0 && matches!(window[j - 1], '"' | '\'' | ')' | ']') {
            j -= 1;
        }
        if j > 0 && matches!(window[j - 1], '.' | '!' | '?' | ';' | ':') {
            cut = Some(i);
        }
    }
    if cut.is_none_or(|c| c * 5 < room * 2) {
        cut = window.iter().rposition(|&c| c == ' ').filter(|&c| c > 0);
    }
    match cut {
        Some(c) => (text[..c].iter().collect::<String>().trim_end().into(), text[c + 1..].iter().collect::<String>().trim_start().into()),
        None => (text[..room].iter().collect(), text[room..].iter().collect()),
    }
}

pub fn paginate(lines: &[Line], target: usize) -> Vec<Page> {
    let max = target + target / 4;
    let mut pages = vec![];
    let mut cur: Vec<Line> = vec![];
    let mut cur_len = 0usize;
    let mut offset = 0u32;

    fn flush(pages: &mut Vec<Page>, cur: &mut Vec<Line>, cur_len: &mut usize, offset: u32) {
        if !cur.is_empty() {
            pages.push(Page { lines: std::mem::take(cur), end: offset });
            *cur_len = 0;
        }
    }

    for line in lines {
        let mut rest = Some(line.clone());
        while let Some(l) = rest.take() {
            let chars: Vec<char> = l.text.chars().collect();
            if !cur.is_empty() && cur_len >= target && l.para {
                flush(&mut pages, &mut cur, &mut cur_len, offset);
            }
            if cur_len + chars.len() <= max {
                cur_len += chars.len() + 1;
                offset += chars.len() as u32 + 1;
                cur.push(l);
            } else if cur_len * 5 >= target * 3 {
                flush(&mut pages, &mut cur, &mut cur_len, offset);
                rest = Some(l);
            } else {
                let (head, tail) = split_at(&chars, target.saturating_sub(cur_len));
                offset += head.chars().count() as u32 + 1;
                cur.push(Line { text: head, para: l.para });
                flush(&mut pages, &mut cur, &mut cur_len, offset);
                if !tail.is_empty() {
                    rest = Some(Line { text: tail, para: false });
                }
            }
        }
    }
    flush(&mut pages, &mut cur, &mut cur_len, offset);
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::chapter_length;

    fn para(s: &str) -> Line {
        Line { text: s.to_string(), para: true }
    }

    #[test]
    fn breaks_at_paragraphs() {
        let lines: Vec<Line> = (0..10).map(|_| para("abcd ".repeat(20).trim_end())).collect();
        let pages = paginate(&lines, 300);
        assert!(pages.len() > 1);
        assert!(pages.iter().all(|p| p.lines.iter().all(|l| l.para)));
        assert_eq!(pages.last().unwrap().end, chapter_length(&lines));
    }

    #[test]
    fn splits_long_paragraphs_at_sentences_preserving_length() {
        let text = "This is a sentence. ".repeat(100);
        let lines = vec![para(text.trim_end())];
        let pages = paginate(&lines, 300);
        assert!(pages.len() > 3);
        for p in &pages {
            let n: usize = p.lines.iter().map(|l| l.text.chars().count()).sum();
            assert!(n <= 375, "page too long: {n}");
            assert!(p.lines.last().unwrap().text.ends_with('.'));
        }
        assert_eq!(pages.last().unwrap().end, chapter_length(&lines));
    }

    #[test]
    fn handles_unbreakable_text() {
        let lines = vec![para(&"x".repeat(1000))];
        let pages = paginate(&lines, 300);
        let total: usize = pages.iter().flat_map(|p| &p.lines).map(|l| l.text.len()).sum();
        assert_eq!(total, 1000);
    }
}
