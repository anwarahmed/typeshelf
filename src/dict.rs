//! The dictionary: Open English WordNet, read straight from its WordNet database files.
//!
//! The files are downloaded once into the cache (`download`) and held in memory while the
//! app runs. The index files are sorted, so a lookup is a binary search plus a slice of
//! the data file at the offset the index gives; nothing is converted or indexed up front
//! beyond the start of each line.

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::store::cache_dir;

/// Open English WordNet, the edition without proper nouns, in the WordNet database format.
pub const URL: &str = "https://github.com/globalwordnet/english-wordnet/releases/download/2025-edition/english-wordnet-2025.zip";
const SHA256: &str = "73355e48f8117a24ca9ebc23ed75b35434e6cd21cc9dd3984e80aff5a5f63636";
/// The folder in the zip, which is also the folder it is unpacked to.
const EDITION: &str = "oewn2025";
pub const CREDIT: &str = "Open English WordNet 2025 · CC BY 4.0";
/// What a download costs, for the message shown while it runs.
pub const DOWNLOAD_SIZE: &str = "10 MB";

/// Words common enough to leave out of the glossary's list for a page, from SCOWL (see
/// `assets/build-common-words.sh`).
const COMMON: &str = include_str!("../assets/common-words.txt");

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pos {
    Noun,
    Verb,
    Adj,
    Adv,
}

impl Pos {
    const ALL: [Pos; 4] = [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv];

    fn file(self) -> &'static str {
        match self {
            Pos::Noun => "noun",
            Pos::Verb => "verb",
            Pos::Adj => "adj",
            Pos::Adv => "adv",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Pos::Noun => "noun",
            Pos::Verb => "verb",
            Pos::Adj => "adjective",
            Pos::Adv => "adverb",
        }
    }

    /// WordNet's rules for taking an inflection off a word: (suffix, replacement).
    fn detachments(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Pos::Noun => &[("s", ""), ("ses", "s"), ("xes", "x"), ("zes", "z"), ("ches", "ch"), ("shes", "sh"), ("men", "man"), ("ies", "y")],
            Pos::Verb => &[("s", ""), ("ies", "y"), ("es", "e"), ("es", ""), ("ed", "e"), ("ed", ""), ("ing", "e"), ("ing", "")],
            Pos::Adj => &[("er", ""), ("est", ""), ("er", "e"), ("est", "e")],
            Pos::Adv => &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sense {
    pub pos: Pos,
    pub def: String,
    pub examples: Vec<String>,
    /// Other words for this sense.
    pub also: Vec<String>,
}

/// The senses of one dictionary form. Looking up "saw" gives two: "saw" and "see".
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub lemma: String,
    pub senses: Vec<Sense>,
}

/// The entries as plain text, for the clipboard: each dictionary form, then its senses
/// numbered under each part of speech, with examples and other words.
pub fn plain(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries {
        if !out.is_empty() {
            out.push('\n');
        }
        out += &format!("{}\n", e.lemma);
        let (mut pos, mut num) = (None, 0);
        for s in &e.senses {
            if pos != Some(s.pos) {
                out += &format!("{}\n", s.pos.label());
                (pos, num) = (Some(s.pos), 0);
            }
            num += 1;
            out += &format!("{num:>2}. {}\n", s.def);
            for ex in &s.examples {
                out += &format!("    \"{ex}\"\n");
            }
            if !s.also.is_empty() {
                out += &format!("    also: {}\n", s.also.join(", "));
            }
        }
    }
    out
}

/// A sorted text file in memory, with the start of each line that isn't part of the
/// license header (those start with a space).
struct Sorted {
    text: Vec<u8>,
    lines: Vec<u32>,
}

impl Sorted {
    fn new(text: Vec<u8>) -> Self {
        let mut lines = vec![];
        let mut start = 0;
        for (i, &b) in text.iter().enumerate() {
            if b == b'\n' {
                if i > start && text[start] != b' ' {
                    lines.push(start as u32);
                }
                start = i + 1;
            }
        }
        if start < text.len() && text[start] != b' ' {
            lines.push(start as u32);
        }
        Sorted { text, lines }
    }

    fn line(&self, i: usize) -> &str {
        let start = self.lines[i] as usize;
        let end = self.text[start..].iter().position(|&b| b == b'\n').map_or(self.text.len(), |n| start + n);
        std::str::from_utf8(&self.text[start..end]).unwrap_or("").trim_end()
    }

    fn key(&self, i: usize) -> &str {
        self.line(i).split(' ').next().unwrap_or("")
    }

    /// The line whose first field is `key`.
    fn find(&self, key: &str) -> Option<&str> {
        let i = self.lower_bound(key);
        (i < self.lines.len() && self.key(i) == key).then(|| self.line(i))
    }

    /// The first line whose key is not less than `key`.
    fn lower_bound(&self, key: &str) -> usize {
        let (mut lo, mut hi) = (0, self.lines.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.key(mid).as_bytes() < key.as_bytes() { lo = mid + 1 } else { hi = mid }
        }
        lo
    }
}

struct Part {
    pos: Pos,
    index: Sorted,
    data: Vec<u8>,
    exceptions: Sorted,
}

pub struct Dictionary {
    parts: Vec<Part>,
    common: HashSet<&'static str>,
}

/// Where the dictionary is unpacked.
pub fn dir() -> PathBuf {
    cache_dir().join("dictionary").join(EDITION)
}

pub fn installed() -> bool {
    Pos::ALL.iter().all(|p| dir().join(format!("data.{}", p.file())).exists())
}

/// The form a word is looked up under: lower case, straight apostrophes, no surrounding
/// punctuation, and WordNet's underscores between the words of a phrase.
pub fn key(word: &str) -> String {
    let w: String = word.chars().map(|c| if matches!(c, '\u{2018}' | '\u{2019}') { '\'' } else { c }).collect::<String>().to_lowercase();
    w.trim_matches(|c: char| !c.is_alphanumeric()).split_whitespace().collect::<Vec<_>>().join("_")
}

impl Dictionary {
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        let mut parts = vec![];
        for pos in Pos::ALL {
            let read = |name: String| fs::read(dir.join(name));
            parts.push(Part {
                pos,
                index: Sorted::new(read(format!("index.{}", pos.file()))?),
                data: read(format!("data.{}", pos.file()))?,
                exceptions: Sorted::new(read(format!("{}.exc", pos.file())).unwrap_or_default()),
            });
        }
        Ok(Dictionary { parts, common: COMMON.lines().filter(|l| !l.starts_with('#')).collect() })
    }

    /// Every dictionary form `word` could be, most likely first: the word itself, the bases
    /// of an irregular form ("went" -> "go"), then what WordNet's suffix rules give.
    pub fn lemmas(&self, word: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for w in candidates(word) {
            let mut add = |part: &Part, base: &str| {
                if !out.iter().any(|o| o == base) && part.index.find(base).is_some() {
                    out.push(base.to_string());
                }
            };
            for part in &self.parts {
                add(part, &w);
            }
            for part in &self.parts {
                for base in part.exceptions.find(&w).map_or("", |l| l).split(' ').skip(1) {
                    add(part, base);
                }
            }
            for part in &self.parts {
                for (suffix, with) in part.pos.detachments() {
                    // A one-letter stem is never meant: "is" is not the letter "i".
                    if let Some(stem) = w.strip_suffix(suffix).filter(|s| s.chars().count() > 1) {
                        add(part, &format!("{stem}{with}"));
                    }
                }
            }
            if !out.is_empty() {
                break;
            }
        }
        out
    }

    /// Worth listing in the glossary: in the dictionary, and neither it nor a form it comes
    /// from ("daughters" from "daughter") is a common word.
    pub fn uncommon(&self, word: &str) -> bool {
        let k = key(word);
        if k.len() < 3 || k.chars().any(|c| c.is_ascii_digit()) || self.common.contains(k.as_str()) {
            return false;
        }
        let lemmas = self.lemmas(word);
        !lemmas.is_empty() && !lemmas.iter().any(|l| self.common.contains(l.as_str()))
    }

    /// All senses of `word`, grouped by dictionary form.
    pub fn define(&self, word: &str) -> Vec<Entry> {
        self.lemmas(word)
            .into_iter()
            .map(|lemma| {
                let mut senses = vec![];
                for part in &self.parts {
                    let Some(line) = part.index.find(&lemma) else { continue };
                    let fields: Vec<&str> = line.split(' ').collect();
                    let count = fields.get(2).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
                    for offset in &fields[fields.len().saturating_sub(count)..] {
                        if let Some(s) = offset.parse::<usize>().ok().and_then(|o| sense(&part.data, o, part.pos, &lemma)) {
                            senses.push(s);
                        }
                    }
                }
                Entry { lemma: lemma.replace('_', " "), senses }
            })
            .filter(|e| !e.senses.is_empty())
            .collect()
    }

    /// Up to `limit` dictionary forms starting with `prefix`, in alphabetical order.
    pub fn complete(&self, prefix: &str, limit: usize) -> Vec<String> {
        let prefix = key(prefix);
        if prefix.is_empty() {
            return vec![];
        }
        let mut out: Vec<String> = vec![];
        for part in &self.parts {
            let mut i = part.index.lower_bound(&prefix);
            while i < part.index.lines.len() && part.index.key(i).starts_with(&prefix) && out.len() < limit * 4 {
                out.push(part.index.key(i).to_string());
                i += 1;
            }
        }
        out.sort();
        out.dedup();
        out.truncate(limit);
        out.into_iter().map(|w| w.replace('_', " ")).collect()
    }
}

/// Spellings to try for a word from a book: as written, without a possessive, and with
/// a hyphen read as a space ("to-morrow" is "tomorrow" in WordNet; "well-known" is listed).
fn candidates(word: &str) -> Vec<String> {
    let k = key(word);
    let mut out = vec![k.clone()];
    if let Some(stem) = k.strip_suffix("'s").or_else(|| k.strip_suffix('\'')) {
        out.push(stem.to_string());
    }
    if k.contains('-') {
        out.push(k.replace('-', "_"));
        out.push(k.replace('-', ""));
    }
    out.retain(|w| !w.is_empty());
    out
}

/// Reads the synset at `offset` in a data file.
fn sense(data: &[u8], offset: usize, pos: Pos, lemma: &str) -> Option<Sense> {
    let rest = data.get(offset..)?;
    let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
    let line = std::str::from_utf8(&rest[..end]).ok()?.trim_end();
    let (head, gloss) = line.split_once(" | ").unwrap_or((line, ""));
    let fields: Vec<&str> = head.split(' ').collect();
    let words = usize::from_str_radix(fields.get(3)?, 16).ok()?;
    let also = (0..words)
        .filter_map(|i| fields.get(4 + 2 * i))
        // Adjectives can carry a position marker: "galore(ip)".
        .map(|w| w.split('(').next().unwrap_or(w))
        .filter(|w| !w.eq_ignore_ascii_case(lemma))
        .map(|w| w.replace('_', " "))
        .collect();
    let (def, examples) = split_gloss(gloss);
    Some(Sense { pos, def, examples, also })
}

/// A gloss is a definition followed by quoted examples, separated by semicolons.
fn split_gloss(gloss: &str) -> (String, Vec<String>) {
    let gloss = gloss.trim();
    let start = if gloss.starts_with('"') { Some(0) } else { gloss.find("; \"").map(|i| i + 2) };
    let Some(start) = start else { return (gloss.to_string(), vec![]) };
    let def = gloss[..start].trim_end_matches([' ', ';']).to_string();
    // An example can name its source after the closing quote: "...nasty" - George Meredith
    let examples = gloss[start..]
        .split("; \"")
        .map(|e| e.trim().trim_start_matches('"').replace("\" - ", " — ").trim_end_matches('"').trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    (def, examples)
}

/// Downloads the dictionary, checks it, and unpacks it into `dir()`. Blocking; run it off
/// the UI thread.
pub fn download() -> Result<(), String> {
    let zip = crate::library::fetch(URL)?;
    let sum: String = Sha256::digest(&zip).iter().map(|b| format!("{b:02x}")).collect();
    if sum != SHA256 {
        return Err("the dictionary download did not match its checksum".into());
    }
    let parent = dir().parent().map(Path::to_path_buf).unwrap_or_default();
    let tmp = parent.join(format!(".{EDITION}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let unpacked = (|| {
        for (name, bytes) in unzip(&zip)? {
            // Only the files a lookup reads; `index.sense` is a third of the unpacked size.
            if let Some(file) = name.strip_prefix(&format!("{EDITION}/"))
                && (file.starts_with("index.") || file.starts_with("data.") || file.ends_with(".exc"))
                && file != "index.sense"
            {
                fs::write(tmp.join(file), bytes).map_err(|e| e.to_string())?;
            }
        }
        let _ = fs::remove_dir_all(dir());
        fs::rename(&tmp, dir()).map_err(|e| e.to_string())
    })();
    if unpacked.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    unpacked
}

fn u16_at(b: &[u8], i: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as usize)
}

fn u32_at(b: &[u8], i: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize)
}

/// The files in a zip archive, from its central directory. Handles what the WordNet
/// release uses (stored and deflated entries, no zip64), which is all this needs.
fn unzip(zip: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let bad = || "the dictionary download is not a readable zip file".to_string();
    let eocd = (0..zip.len().saturating_sub(21)).rev().find(|&i| zip[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06])).ok_or_else(bad)?;
    let count = u16_at(zip, eocd + 10).ok_or_else(bad)?;
    let mut at = u32_at(zip, eocd + 16).ok_or_else(bad)?;
    let mut out = vec![];
    for _ in 0..count {
        if !zip.get(at..).is_some_and(|z| z.starts_with(&[0x50, 0x4b, 0x01, 0x02])) {
            return Err(bad());
        }
        let field = |n: usize| u16_at(zip, at + n).ok_or_else(bad);
        let (method, size, name_len) = (field(10)?, u32_at(zip, at + 20).ok_or_else(bad)?, field(28)?);
        let local = u32_at(zip, at + 42).ok_or_else(bad)?;
        let name = String::from_utf8_lossy(zip.get(at + 46..at + 46 + name_len).ok_or_else(bad)?).into_owned();
        at += 46 + name_len + field(30)? + field(32)?;
        let start = local + 30 + u16_at(zip, local + 26).ok_or_else(bad)? + u16_at(zip, local + 28).ok_or_else(bad)?;
        let raw = zip.get(start..start + size).ok_or_else(bad)?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = vec![];
                flate2::read::DeflateDecoder::new(raw).read_to_end(&mut v).map_err(|_| bad())?;
                v
            }
            _ => return Err(bad()),
        };
        if !name.ends_with('/') {
            out.push((name, bytes));
        }
    }
    Ok(out)
}

/// The words a book page is split into for looking up: runs of letters and digits, with
/// apostrophes and hyphens inside a word kept ("don't", "well-known"). Returns
/// `(start, end)` indices into `chars`.
pub fn words(chars: &[char]) -> Vec<(usize, usize)> {
    let inner = |c: char| matches!(c, '\'' | '\u{2019}' | '-');
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_alphanumeric() || (inner(chars[i]) && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric()))) {
            i += 1;
        }
        out.push((start, i));
    }
    out
}

/// Titles written with a full stop before a name.
const TITLES: [&str; 17] = ["mr", "mrs", "ms", "dr", "st", "messrs", "mme", "mlle", "rev", "col", "capt", "gen", "lt", "sgt", "prof", "hon", "sir"];

/// The words on a page that look like names: capitalized every time, and at least once
/// in the middle of a sentence ("said Mr. Bennet"). WordNet knows some names as ordinary
/// words (a bennet is a plant), which the glossary should not list.
pub fn names(chars: &[char], words: &[(usize, usize)]) -> HashSet<String> {
    let mut capitalized: std::collections::HashMap<String, (bool, bool)> = std::collections::HashMap::new();
    for &(s, e) in words {
        let word: String = chars[s..e].iter().collect();
        let before = chars[..s].iter().rposition(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '(' | '[' | '_' | '-' | '\u{2014}'));
        let mid_sentence = before.is_some_and(|b| match chars[b] {
            // A title's full stop doesn't end the sentence: "Mr. Bennet".
            '.' => {
                let title: String = chars[..b].iter().rev().take_while(|c| c.is_alphabetic()).collect();
                TITLES.contains(&title.chars().rev().collect::<String>().to_lowercase().as_str())
            }
            '!' | '?' => false,
            _ => true,
        });
        let e = capitalized.entry(key(&word)).or_insert((true, false));
        e.0 &= word.starts_with(char::is_uppercase);
        e.1 |= mid_sentence;
    }
    capitalized.into_iter().filter(|(_, (always, mid))| *always && *mid).map(|(k, _)| k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A few entries in the real file format, license header included.
    fn tiny() -> Dictionary {
        let header = "  1 This software and database is being provided to you\n  2 under CC BY 4.0.  \n";
        let noun_data = format!(
            "{header}{}{}",
            "00000123 07 n 02 countenance 0 visage 0 001 @ 04041049 n 0000 | the appearance conveyed by a person's face; \"a pleasant countenance\"; \"a stern visage\"  \n",
            "00000300 06 n 01 saw 0 000 | hand tool having a toothed blade  \n"
        );
        let noun_off = |w: &str| noun_data.find(&format!("{w} 0")).map(|i| noun_data[..i].rfind('\n').map_or(0, |n| n + 1)).unwrap();
        let verb_data = format!(
            "{header}{}{}",
            "00000050 32 v 02 see 0 understand 0 000 | perceive by sight  \n",
            "00000200 32 v 04 permit 0 allow 0 let 2 countenance 0 000 | consent to, give permission; permit; \"She permitted her son to visit\"  \n"
        );
        let verb_off = |w: &str| verb_data.find(&format!("{w} 0")).map(|i| verb_data[..i].rfind('\n').map_or(0, |n| n + 1)).unwrap();
        let noun_index = format!(
            "{header}countenance n 1 1 @ 1 0 {:08}  \nface n 1 1 @ 1 0 {:08}  \nsaw n 1 1 @ 1 0 {:08}  \n",
            noun_off("countenance"),
            noun_off("countenance"),
            noun_off("saw")
        );
        let verb_index = format!("{header}countenance v 1 1 @ 1 0 {:08}  \nsee v 1 1 @ 1 0 {:08}  \n", verb_off("permit"), verb_off("see"));
        let part = |pos, index: String, data: String, exc: &str| Part {
            pos,
            index: Sorted::new(index.into_bytes()),
            data: data.into_bytes(),
            exceptions: Sorted::new(exc.as_bytes().to_vec()),
        };
        Dictionary {
            parts: vec![
                part(Pos::Noun, noun_index, noun_data, ""),
                part(Pos::Verb, verb_index, verb_data, "saw see\nseen see\n"),
                part(Pos::Adj, header.into(), header.into(), ""),
                part(Pos::Adv, header.into(), header.into(), ""),
            ],
            common: ["face"].into(),
        }
    }

    #[test]
    fn defines_a_word_with_examples_and_synonyms() {
        let d = tiny();
        let e = d.define("Countenance");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].lemma, "countenance");
        let noun = &e[0].senses[0];
        assert_eq!((noun.pos, noun.def.as_str()), (Pos::Noun, "the appearance conveyed by a person's face"));
        assert_eq!(noun.examples, ["a pleasant countenance", "a stern visage"]);
        assert_eq!(noun.also, ["visage"]);
        let verb = &e[0].senses[1];
        assert_eq!((verb.pos, verb.def.as_str()), (Pos::Verb, "consent to, give permission; permit"));
        assert_eq!(verb.also, ["permit", "allow", "let"]);
    }

    #[test]
    fn copies_a_definition_as_plain_text() {
        let text = plain(&tiny().define("countenance"));
        assert_eq!(
            text,
            "countenance\nnoun\n 1. the appearance conveyed by a person's face\n    \"a pleasant countenance\"\n    \"a stern visage\"\n    also: visage\n\
             verb\n 1. consent to, give permission; permit\n    \"She permitted her son to visit\"\n    also: permit, allow, let\n"
        );
    }

    #[test]
    fn reduces_inflections_and_irregular_forms() {
        let d = tiny();
        assert_eq!(d.lemmas("countenances"), ["countenance"]);
        assert_eq!(d.lemmas("countenanced"), ["countenance"]);
        assert_eq!(d.lemmas("seen"), ["see"]);
        // As written first, then what it may be a form of.
        assert_eq!(d.lemmas("saw"), ["saw", "see"]);
        assert_eq!(d.define("saw").iter().map(|e| e.lemma.as_str()).collect::<Vec<_>>(), ["saw", "see"]);
        assert_eq!(d.lemmas("\u{201C}Countenance\u{2019}s,"), ["countenance"]);
        assert!(d.lemmas("zebra").is_empty());
        // An irregular form comes before what the suffix rules make of it.
        assert_eq!(d.lemmas("sees"), ["see"]);
    }

    #[test]
    fn uncommon_words_leave_out_common_and_unknown_ones() {
        let mut d = tiny();
        assert!(d.uncommon("countenance"));
        assert!(!d.uncommon("face"));
        assert!(!d.uncommon("zebra"));
        // Common in any of its forms: "seen" is "see".
        d.common.insert("see");
        assert!(!d.uncommon("seen"));
    }

    #[test]
    fn spots_names_on_a_page() {
        let chars: Vec<char> = "\"My dear Mr. Bennet,\" said his lady. Bennet replied. Impatiently, she cried. Cried Kitty.".chars().collect();
        let mut got: Vec<String> = names(&chars, &words(&chars)).into_iter().collect();
        got.sort();
        // "Impatiently" only starts a sentence; "cried" is also written in lower case.
        assert_eq!(got, ["bennet", "kitty", "mr"]);
    }

    #[test]
    fn completes_a_prefix() {
        let d = tiny();
        assert_eq!(d.complete("s", 10), ["saw", "see"]);
        assert_eq!(d.complete("co", 10), ["countenance"]);
        assert!(d.complete("", 10).is_empty());
    }

    #[test]
    fn splits_a_page_into_words() {
        let chars: Vec<char> = "\"Don't,\" she said--well-known 'twas".chars().collect();
        let got: Vec<String> = words(&chars).into_iter().map(|(s, e)| chars[s..e].iter().collect()).collect();
        assert_eq!(got, ["Don't", "she", "said", "well-known", "twas"]);
    }

    #[test]
    fn glosses_without_examples() {
        assert_eq!(split_gloss("hand tool"), ("hand tool".into(), vec![]));
        assert_eq!(split_gloss("\"only an example\""), (String::new(), vec!["only an example".into()]));
        assert_eq!(split_gloss("pleasant; \"what a nice fellow\" - George Meredith; \"a nice day\"").1, ["what a nice fellow — George Meredith", "a nice day"]);
    }

    #[test]
    fn reads_stored_and_deflated_zip_entries() {
        use std::io::Write;
        let mut deflated = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
        deflated.write_all(b"hello hello hello").unwrap();
        let deflated = deflated.finish().unwrap();
        let mut zip = vec![];
        let mut central = vec![];
        for (name, method, data) in [("a/one", 0u16, b"plain".to_vec()), ("a/two", 8, deflated)] {
            let offset = zip.len() as u32;
            zip.extend([0x50, 0x4b, 0x03, 0x04]);
            zip.extend([0u8; 22]);
            zip.extend((name.len() as u16).to_le_bytes());
            zip.extend(0u16.to_le_bytes());
            zip.extend(name.as_bytes());
            zip.extend(&data);
            central.extend([0x50, 0x4b, 0x01, 0x02]);
            central.extend([0u8; 6]);
            central.extend(method.to_le_bytes());
            central.extend([0u8; 8]);
            central.extend((data.len() as u32).to_le_bytes());
            central.extend([0u8; 4]);
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let at = zip.len() as u32;
        zip.extend(&central);
        zip.extend([0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 2, 0, 2, 0]);
        zip.extend((central.len() as u32).to_le_bytes());
        zip.extend(at.to_le_bytes());
        zip.extend([0, 0]);
        let files = unzip(&zip).unwrap();
        assert_eq!(files, [("a/one".to_string(), b"plain".to_vec()), ("a/two".to_string(), b"hello hello hello".to_vec())]);
    }
}
