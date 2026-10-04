//! The typing session: what has been typed against a page of text, and how well.

use std::collections::HashMap;

use crate::normalize::fold;

/// Pauses longer than this don't count towards typing time.
const IDLE_CAP_MS: u64 = 5000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    Pending,
    Correct,
    Wrong,
    /// Not typeable on a keyboard; the cursor jumps over it.
    Skipped,
}

pub struct Cell {
    pub ch: char,
    /// The key that types it, `None` if there isn't one.
    pub key: Option<char>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub wpm: f32,
    pub acc: f32,
    pub ms: u64,
    pub chars: u32,
    pub mistakes: u32,
}

pub struct Session {
    pub cells: Vec<Cell>,
    pub marks: Vec<Mark>,
    /// Cells that were mistyped at some point, even if corrected since.
    pub had_error: Vec<bool>,
    pub pos: usize,
    /// Cells before this were typed in an earlier sitting; they are shown as done but
    /// don't count towards this session's stats and can't be deleted.
    pub resumed: usize,
    pub keystrokes: u32,
    pub mistakes: u32,
    /// Mistake count per expected key.
    pub missed: HashMap<char, u32>,
    active_ms: u64,
    last_ms: Option<u64>,
    stop_on_error: bool,
    space_for_enter: bool,
}

impl Session {
    pub fn new(text: &str, stop_on_error: bool, space_for_enter: bool) -> Self {
        let cells: Vec<Cell> = text.chars().map(|ch| Cell { ch, key: fold(ch) }).collect();
        let n = cells.len();
        let mut s = Session {
            cells,
            marks: vec![Mark::Pending; n],
            had_error: vec![false; n],
            pos: 0,
            resumed: 0,
            keystrokes: 0,
            mistakes: 0,
            missed: HashMap::new(),
            active_ms: 0,
            last_ms: None,
            stop_on_error,
            space_for_enter,
        };
        s.skip_untypeable();
        s
    }

    fn skip_untypeable(&mut self) {
        while self.pos < self.cells.len() && self.cells[self.pos].key.is_none() {
            self.marks[self.pos] = Mark::Skipped;
            self.pos += 1;
        }
    }

    /// Starts the session part-way through the page, as if `pos` cells were already typed.
    pub fn resume_at(&mut self, pos: usize) {
        let pos = pos.min(self.cells.len().saturating_sub(1));
        for i in 0..pos {
            self.marks[i] = if self.cells[i].key.is_some() { Mark::Correct } else { Mark::Skipped };
        }
        self.pos = pos;
        self.skip_untypeable();
        self.resumed = self.pos;
    }

    pub fn done(&self) -> bool {
        self.pos >= self.cells.len()
    }

    pub fn started(&self) -> bool {
        self.last_ms.is_some()
    }

    fn tick(&mut self, now_ms: u64) {
        if let Some(last) = self.last_ms {
            self.active_ms += now_ms.saturating_sub(last).min(IDLE_CAP_MS);
        }
        self.last_ms = Some(now_ms);
    }

    pub fn type_char(&mut self, c: char, now_ms: u64) {
        if self.done() {
            return;
        }
        self.tick(now_ms);
        let want = self.cells[self.pos].key.unwrap_or(' ');
        let got = fold(c).unwrap_or(c);
        let ok = got == want || (want == '\n' && got == ' ' && self.space_for_enter);
        self.keystrokes += 1;
        if ok {
            self.marks[self.pos] = Mark::Correct;
            self.pos += 1;
        } else {
            self.mistakes += 1;
            *self.missed.entry(want).or_default() += 1;
            self.had_error[self.pos] = true;
            self.marks[self.pos] = Mark::Wrong;
            if !self.stop_on_error {
                self.pos += 1;
            }
        }
        self.skip_untypeable();
    }

    pub fn backspace(&mut self) {
        if self.done() {
            return;
        }
        let Some(target) = (self.resumed..self.pos).rev().find(|&i| self.cells[i].key.is_some()) else {
            self.marks[self.pos] = Mark::Pending;
            return;
        };
        for m in &mut self.marks[target..=self.pos] {
            *m = Mark::Pending;
        }
        self.pos = target;
    }

    /// Deletes back to the start of the current word.
    pub fn backspace_word(&mut self) {
        let boundary = |s: &Self| s.pos <= s.resumed || matches!(s.cells[s.pos - 1].ch, ' ' | '\n');
        self.backspace();
        while !self.done() && !boundary(self) {
            let before = self.pos;
            self.backspace();
            if self.pos == before {
                break;
            }
        }
    }

    /// Typing time so far, including the pause since the last key (capped).
    pub fn elapsed_ms(&self, now_ms: u64) -> u64 {
        match self.last_ms {
            Some(last) if !self.done() => self.active_ms + now_ms.saturating_sub(last).min(IDLE_CAP_MS),
            _ => self.active_ms,
        }
    }

    pub fn stats(&self, now_ms: u64) -> Stats {
        let ms = self.elapsed_ms(now_ms);
        let correct = self.marks[self.resumed..].iter().filter(|&&m| m == Mark::Correct).count() as u32;
        let chars = self.cells[self.resumed..].iter().filter(|c| c.key.is_some()).count() as u32;
        let minutes = ms as f32 / 60_000.0;
        Stats {
            wpm: if minutes > 0.0 { correct as f32 / 5.0 / minutes } else { 0.0 },
            acc: if self.keystrokes > 0 { (self.keystrokes - self.mistakes) as f32 / self.keystrokes as f32 * 100.0 } else { 100.0 },
            ms,
            chars,
            mistakes: self.mistakes,
        }
    }

    /// Fraction of the page typed, 0..=1.
    pub fn progress(&self) -> f64 {
        if self.cells.is_empty() { 1.0 } else { self.pos as f64 / self.cells.len() as f64 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(s: &mut Session, text: &str, start: u64) {
        for (i, c) in text.chars().enumerate() {
            s.type_char(c, start + i as u64 * 200);
        }
    }

    #[test]
    fn perfect_run() {
        let mut s = Session::new("hello world", false, true);
        type_str(&mut s, "hello world", 0);
        assert!(s.done());
        let st = s.stats(99_999);
        assert_eq!(st.acc, 100.0);
        assert_eq!(st.ms, 2000);
        assert!((st.wpm - 66.0).abs() < 0.1);
    }

    #[test]
    fn mistakes_and_backspace() {
        let mut s = Session::new("cat", false, true);
        type_str(&mut s, "cx", 0);
        assert_eq!(s.marks[1], Mark::Wrong);
        s.backspace();
        assert_eq!((s.pos, s.marks[1]), (1, Mark::Pending));
        type_str(&mut s, "at", 1000);
        assert!(s.done() && s.had_error[1]);
        assert_eq!(s.stats(0).mistakes, 1);
        assert_eq!(s.missed[&'a'], 1);
        assert_eq!(s.stats(0).acc, 75.0);
    }

    #[test]
    fn stop_on_error_holds_cursor() {
        let mut s = Session::new("ab", true, true);
        s.type_char('x', 0);
        assert_eq!((s.pos, s.marks[0]), (0, Mark::Wrong));
        s.type_char('a', 1);
        assert_eq!((s.pos, s.marks[0]), (1, Mark::Correct));
    }

    #[test]
    fn folds_accents_dashes_and_newlines() {
        let mut s = Session::new("café\u{2014}x\ny", false, true);
        type_str(&mut s, "cafe-x y", 0);
        assert!(s.done());
        assert_eq!(s.mistakes, 0);
        let mut strict = Session::new("a\nb", false, false);
        type_str(&mut strict, "a ", 0);
        assert_eq!(strict.mistakes, 1);
    }

    #[test]
    fn skips_untypeable_both_ways() {
        let mut s = Session::new("£a λ b", false, true);
        assert_eq!(s.pos, 1);
        type_str(&mut s, "a ", 0);
        assert_eq!(s.pos, 4);
        s.backspace();
        assert_eq!(s.pos, 2);
        s.backspace();
        s.backspace();
        assert_eq!(s.pos, 1);
        type_str(&mut s, "a  b", 0);
        assert!(s.done());
        assert_eq!(s.stats(0).chars, 4);
    }

    #[test]
    fn word_delete() {
        let mut s = Session::new("one two three", false, true);
        type_str(&mut s, "one tw", 0);
        s.backspace_word();
        assert_eq!(s.pos, 4);
        s.backspace_word();
        assert_eq!(s.pos, 0);
    }

    #[test]
    fn resumes_part_way() {
        let mut s = Session::new("one two", false, true);
        s.resume_at(4);
        assert_eq!((s.pos, s.marks[3], s.marks[4]), (4, Mark::Correct, Mark::Pending));
        s.backspace();
        s.backspace_word();
        assert_eq!(s.pos, 4);
        type_str(&mut s, "two", 0);
        assert!(s.done());
        let st = s.stats(0);
        assert_eq!((st.chars, st.acc), (3, 100.0));
        assert!((st.wpm - 90.0).abs() < 0.1);
    }

    #[test]
    fn idle_time_is_capped() {
        let mut s = Session::new("ab", false, true);
        s.type_char('a', 0);
        s.type_char('b', 60_000);
        assert_eq!(s.stats(0).ms, 5000);
    }
}
