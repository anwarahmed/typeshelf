//! Text normalization: turn typographic characters into things a keyboard can produce.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::{decompose_canonical, is_combining_mark};

/// Rewrites text so that as much of it as possible is typeable. Applied once, when a
/// book is parsed.
pub fn normalize_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.nfc() {
        match c {
            '\u{FEFF}' | '\u{200B}'..='\u{200D}' | '\u{00AD}' | '\u{2060}' => {}
            '\u{00A0}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\t' => out.push(' '),
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => out.push('\''),
            '\u{201C}'..='\u{201F}' | '\u{2033}' => out.push('"'),
            '\u{2026}' => out.push_str("..."),
            'æ' => out.push_str("ae"),
            'Æ' => out.push_str("AE"),
            'œ' => out.push_str("oe"),
            'Œ' => out.push_str("OE"),
            'ß' => out.push_str("ss"),
            'ſ' => out.push('s'),
            '½' => out.push_str("1/2"),
            '¼' => out.push_str("1/4"),
            '¾' => out.push_str("3/4"),
            'ﬁ' => out.push_str("fi"),
            'ﬂ' => out.push_str("fl"),
            _ => out.push(c),
        }
    }
    out
}

/// The key that types a character: itself for printable ASCII, the base letter for
/// accented ones, a hyphen for dashes. `None` when no key produces it (Greek, symbols,
/// ...) - such characters are skipped over while typing.
pub fn fold(c: char) -> Option<char> {
    if c == '\n' || (' '..='~').contains(&c) {
        return Some(c);
    }
    let mapped = match c {
        '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
        '«' | '»' | '\u{201C}' | '\u{201D}' => '"',
        '\u{2018}' | '\u{2019}' | '´' => '\'',
        '×' => 'x',
        'ø' => 'o',
        'Ø' => 'O',
        'ł' => 'l',
        'Ł' => 'L',
        'đ' => 'd',
        'Đ' => 'D',
        _ => {
            let mut base = None;
            let mut extra = false;
            decompose_canonical(c, |d| {
                if is_combining_mark(d) {
                } else if base.is_none() {
                    base = Some(d)
                } else {
                    extra = true
                }
            });
            match base {
                Some(b) if !extra && (' '..='~').contains(&b) => b,
                _ => return None,
            }
        }
    };
    Some(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_typography() {
        assert_eq!(normalize_text("\u{201C}Hi\u{201D}\u{2026} it\u{2019}s"), "\"Hi\"... it's");
        assert_eq!(normalize_text("C\u{00E6}sar\u{00A0}\u{FEFF}x"), "Caesar x");
    }

    #[test]
    fn folds_to_keys() {
        assert_eq!(fold('a'), Some('a'));
        assert_eq!(fold('é'), Some('e'));
        assert_eq!(fold('Ñ'), Some('N'));
        assert_eq!(fold('\u{2014}'), Some('-'));
        assert_eq!(fold('λ'), None);
        assert_eq!(fold('£'), None);
        assert_eq!(fold('\u{0305}'), None);
    }
}
