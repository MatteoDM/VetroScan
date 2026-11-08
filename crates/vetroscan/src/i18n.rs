// SPDX-License-Identifier: GPL-2.0-or-later
//! The app's texts in Italian, its first language, and in English.
//!
//! Gettext style: the key of a text is the Italian text itself, so the code
//! reads as the app looks and Italian needs no catalog. [`tr`] gives a text
//! in the language in use; [`trf`] also fills in its `{name}` places. The
//! English texts are in `i18n/en.json`, Italian → English; the tests below
//! check that every text the code asks for has its English, that none is
//! left over, and that no text for people is written in the code without
//! going through here.
//!
//! The language is chosen in the Preferences: *Automatica* (English when
//! the system's language is English, Italian otherwise), *Italiano* or
//! *English*.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;

/// A language of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    It,
    En,
}

/// The language preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangChoice {
    /// The system's: English if it is English, Italian otherwise.
    Auto,
    It,
    En,
}

impl LangChoice {
    pub const ALL: [LangChoice; 3] = [Self::Auto, Self::It, Self::En];

    /// The name it is saved with.
    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "automatica",
            Self::It => "italiano",
            Self::En => "english",
        }
    }

    /// The choice saved as `key` (unknown: automatic).
    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|c| c.key() == key)
            .unwrap_or(Self::Auto)
    }

    /// The language it means on this system.
    pub fn resolve(self) -> Lang {
        match self {
            Self::Auto => system(),
            Self::It => Lang::It,
            Self::En => Lang::En,
        }
    }
}

impl Display for LangChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Each language by its own name, so it can be found in any.
        f.write_str(match self {
            Self::Auto => tr("Automatica (lingua del sistema)"),
            Self::It => "Italiano",
            Self::En => "English",
        })
    }
}

/// The language of the system (Italian in tests, whatever the machine).
pub fn system() -> Lang {
    if cfg!(test) {
        return Lang::It;
    }
    from_locale(sys_locale::get_locale().as_deref())
}

/// English for an English locale ("en", "en-GB", "en_US.UTF-8"), Italian
/// for any other.
pub fn from_locale(locale: Option<&str>) -> Lang {
    match locale {
        Some(l) if l.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("en")) => Lang::En,
        _ => Lang::It,
    }
}

const ENGLISH: &str = include_str!("../i18n/en.json");

/// Italian → English, read once.
fn english() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let map: std::collections::BTreeMap<String, String> =
            serde_json::from_str(ENGLISH).expect("i18n/en.json");
        map.into_iter()
            .map(|(it, en)| {
                let it: &'static str = Box::leak(it.into_boxed_str());
                let en: &'static str = Box::leak(en.into_boxed_str());
                (it, en)
            })
            .collect()
    })
}

#[cfg(not(test))]
static CURRENT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    // Tests run side by side, some in English: each thread its own.
    static CURRENT: std::cell::Cell<Lang> = const { std::cell::Cell::new(Lang::It) };
}

/// Uses `lang` from now on.
pub fn set(lang: Lang) {
    #[cfg(not(test))]
    CURRENT.store(lang as u8, std::sync::atomic::Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.set(lang));
}

/// The language in use.
pub fn current() -> Lang {
    #[cfg(not(test))]
    return match CURRENT.load(std::sync::atomic::Ordering::Relaxed) {
        1 => Lang::En,
        _ => Lang::It,
    };
    #[cfg(test)]
    CURRENT.with(|c| c.get())
}

/// The text `it` (Italian) in the language in use.
///
/// An Italian word with two meanings in English carries its meaning in
/// front, gettext's `msgctxt`: `tr("undo|Annulla")` is "Annulla" in Italian
/// and the English of the key `undo|Annulla` ("Undo"), while
/// `tr("Annulla")` stays "Cancel".
pub fn tr(it: &'static str) -> &'static str {
    let italian = it.split_once('|').map_or(it, |(_, text)| text);
    match current() {
        Lang::It => italian,
        Lang::En => english().get(it).copied().unwrap_or(italian),
    }
}

/// A number written with a decimal point, with a decimal comma in Italian.
pub fn decimal(number: String) -> String {
    match current() {
        Lang::It => number.replace('.', ","),
        Lang::En => number,
    }
}

/// The text `it` in the language in use, its `{name}` places filled with
/// `args`.
pub fn trf(it: &'static str, args: &[(&str, &dyn Display)]) -> String {
    let mut text = tr(it).to_string();
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), &value.to_string());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The app's sources that show text, with the text of each.
    fn sources() -> Vec<(&'static str, &'static str)> {
        vec![
            ("main.rs", include_str!("main.rs")),
            ("editor.rs", include_str!("editor.rs")),
            ("worker.rs", include_str!("worker.rs")),
            ("wia.rs", include_str!("wia.rs")),
            ("access.rs", include_str!("access.rs")),
            ("prefs.rs", include_str!("prefs.rs")),
            ("viewer.rs", include_str!("viewer.rs")),
            ("sheet.rs", include_str!("sheet.rs")),
            ("curve.rs", include_str!("curve.rs")),
            ("histogram.rs", include_str!("histogram.rs")),
            ("i18n.rs", include_str!("i18n.rs")),
        ]
    }

    /// The code of a source, without its unit tests.
    fn code(source: &str) -> &str {
        source
            .find("#[cfg(test)]\nmod tests")
            .or_else(|| source.find("#[cfg(test)]\r\nmod tests"))
            .map_or(source, |end| &source[..end])
    }

    /// A Rust string literal starting at `s` (after its opening quote):
    /// its value and its length in the source.
    fn literal(s: &str) -> (String, usize) {
        let mut out = String::new();
        let mut chars = s.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return (out, i + 1),
                '\\' => match chars.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, '\\')) => out.push('\\'),
                    Some((_, '"')) => out.push('"'),
                    Some((_, '\'')) => out.push('\''),
                    Some((_, '\n' | '\r')) => {
                        // A line continuation: the next line's indent goes.
                        while chars.peek().is_some_and(|(_, c)| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    Some((_, 'u')) => {
                        let rest: String = chars
                            .by_ref()
                            .map(|(_, c)| c)
                            .take_while(|&c| c != '}')
                            .collect();
                        let hex = rest.trim_start_matches('{');
                        if let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                        {
                            out.push(c);
                        }
                    }
                    Some((_, c)) => out.push(c),
                    None => {}
                },
                c => out.push(c),
            }
        }
        (out, s.len())
    }

    /// Every literal of the code: where it starts, its value, and the code
    /// just before it.
    fn literals(source: &str) -> Vec<(usize, String, String)> {
        let code = code(source);
        let mut out = Vec::new();
        let mut i = 0;
        let bytes = code.as_bytes();
        while i < bytes.len() {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    // A comment, to the end of the line.
                    i = code[i..].find('\n').map_or(bytes.len(), |n| i + n);
                }
                b'\'' => {
                    // A char literal ('"' included) or a lifetime.
                    let next = code[i + 1..].chars().next().map_or(1, char::len_utf8);
                    i += if bytes.get(i + 1 + next) == Some(&b'\'') {
                        next + 2
                    } else if bytes.get(i + 1) == Some(&b'\\') {
                        code[i + 2..].find('\'').map_or(1, |n| n + 3)
                    } else {
                        1
                    };
                }
                b'"' => {
                    let raw = i > 0 && bytes[i - 1] == b'r';
                    let (value, len) = if raw {
                        let end = code[i + 1..].find('"').unwrap_or(0);
                        (code[i + 1..i + 1 + end].to_string(), end + 2)
                    } else {
                        literal(&code[i + 1..])
                    };
                    let before = code[..i].trim_end();
                    let from = (before.len().saturating_sub(40)..=before.len())
                        .find(|&b| before.is_char_boundary(b))
                        .unwrap_or(0);
                    let before = before[from..].to_string();
                    out.push((i, value, before));
                    i += len + usize::from(!raw);
                }
                _ => i += 1,
            }
        }
        out
    }

    pub(super) fn asked_for_survey() -> BTreeSet<String> {
        asked()
    }

    pub(super) fn sources_for_survey() -> Vec<(&'static str, &'static str)> {
        sources()
    }

    pub(super) fn literals_for_survey(source: &str) -> Vec<(usize, String, String)> {
        literals(source)
    }

    /// Whether the literal is the text of a [`tr`] or [`trf`] call.
    fn translated(before: &str) -> bool {
        before.ends_with("tr(") || before.ends_with("trf(")
    }

    /// Every text the code asks for.
    fn asked() -> BTreeSet<String> {
        sources()
            .into_iter()
            .flat_map(|(_, s)| literals(s))
            .filter(|(_, _, before)| translated(before))
            .map(|(_, v, _)| v)
            .collect()
    }

    fn places(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            match rest[start..].find('}') {
                Some(end) => {
                    out.insert(rest[start..start + end + 1].to_string());
                    rest = &rest[start + end + 1..];
                }
                None => break,
            }
        }
        out
    }

    #[test]
    fn every_text_has_its_english_and_none_is_left_over() {
        let asked = asked();
        let known: BTreeSet<String> = english().keys().map(|k| k.to_string()).collect();
        let missing: Vec<_> = asked.difference(&known).collect();
        let unused: Vec<_> = known.difference(&asked).collect();
        assert!(missing.is_empty(), "without English: {missing:#?}");
        assert!(unused.is_empty(), "English never asked for: {unused:#?}");
        for (it, en) in english() {
            assert_eq!(places(it), places(en), "{it} → {en}");
            assert!(!en.trim().is_empty(), "{it}");
        }
    }

    /// Whether a literal looks like text for people: two words, or one
    /// capitalised word. A word has two letters or more and none of the
    /// marks of names in code, paths or keys (`_ - . / \ = $`); the `{…}`
    /// places do not count.
    fn wordy(s: &str) -> bool {
        let mut plain = String::new();
        let mut depth = 0;
        for c in s.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                c if depth == 0 => plain.push(c),
                _ => {}
            }
        }
        let words: Vec<&str> = plain
            .split_whitespace()
            .filter(|w| !w.contains(['_', '-', '.', '/', '\\', '=', '$', '%']))
            .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 2)
            .collect();
        let tokens = plain.split_whitespace().count();
        words.len() >= 2
            || (tokens == 1
                && words.len() == 1
                && words[0].starts_with(char::is_uppercase)
                && words[0].chars().count() >= 3)
    }

    #[test]
    fn what_looks_like_text() {
        assert!(wordy("Scansiona foto"));
        assert!(wordy("Anteprima"));
        assert!(wordy("{n} file a 16 bit in {dir}"));
        assert!(!wordy("aggiungi-pagina"));
        assert!(!wordy("impostazioni.json"));
        assert!(!wordy("{now:.3} {line}"));
        assert!(!wordy("HKEY_LOCAL_MACHINE"));
        assert!(!wordy("MB"));
    }

    #[test]
    fn no_text_for_people_skips_the_translation() {
        // What is not for people, and is fine as it is.
        let calls_not_shown = [
            "log(",
            "expect(",
            "panic!(",
            "var_os(",
            "debug_assert",
            "assert",
            "include_str!(",
            "eprintln!(",
            ".name(",
            "Builder::new()",
            "record(",
        ];
        let mut found = Vec::new();
        for (file, source) in sources() {
            if file == "i18n.rs" {
                continue;
            }
            for (at, value, before) in literals(source) {
                if translated(&before) || !wordy(&value) {
                    continue;
                }
                let line_start = source[..at].rfind('\n').map_or(0, |n| n + 1);
                let line_end = source[at..].find('\n').map_or(source.len(), |n| at + n);
                let line = &source[line_start..line_end];
                let previous = source[..line_start.saturating_sub(1)]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("");
                if line.contains("// not translated") || previous.contains("// not translated") {
                    continue;
                }
                if calls_not_shown.iter().any(|c| before.contains(c)) {
                    continue;
                }
                let n = source[..at].matches('\n').count() + 1;
                found.push(format!("{file}:{n}: {value}"));
            }
        }
        assert!(found.is_empty(), "texts not through tr()/trf(): {found:#?}");
    }

    #[test]
    fn english_and_italian() {
        set(Lang::En);
        assert_eq!(tr("Anteprima"), "Preview");
        assert_eq!(trf("Scansiona {n} pagine", &[("n", &3)]), "Scan 3 pages");
        // A text without English stays Italian.
        assert_eq!(tr("una parola che non c'è"), "una parola che non c'è");
        set(Lang::It);
        assert_eq!(tr("Anteprima"), "Anteprima");
        assert_eq!(
            trf("Scansiona {n} pagine", &[("n", &3)]),
            "Scansiona 3 pagine"
        );
    }

    #[test]
    fn the_system_language_decides_automatic() {
        assert_eq!(from_locale(Some("en-GB")), Lang::En);
        assert_eq!(from_locale(Some("en_US.UTF-8")), Lang::En);
        assert_eq!(from_locale(Some("EN")), Lang::En);
        assert_eq!(from_locale(Some("it-IT")), Lang::It);
        assert_eq!(from_locale(Some("de-DE")), Lang::It);
        assert_eq!(from_locale(None), Lang::It);
        assert_eq!(LangChoice::from_key("english").resolve(), Lang::En);
        assert_eq!(LangChoice::from_key("boh"), LangChoice::Auto);
    }
}
#[cfg(test)]
mod survey {
    /// Lists every literal with letters not through tr()/trf(), for a
    /// look by hand: `cargo test -p vetroscan i18n::survey -- --ignored --nocapture`.
    #[test]
    #[ignore = "a list to read, not a check"]
    fn short_texts() {
        for (file, source) in super::tests::sources_for_survey() {
            for (at, value, before) in super::tests::literals_for_survey(source) {
                let letters = value.chars().filter(|c| c.is_lowercase()).count();
                if letters < 2 || before.ends_with("tr(") || before.ends_with("trf(") {
                    continue;
                }
                let n = source[..at].matches('\n').count() + 1;
                let line = source.lines().nth(n - 1).unwrap_or("").trim();
                println!(
                    "{file}:{n}: {value:?}  | {}",
                    line.chars().take(70).collect::<String>()
                );
            }
        }
    }
}
#[cfg(test)]
mod keys {
    /// Writes every text the code asks for to `I18N_KEYS` (a JSON list), to
    /// start the English catalog of new texts.
    #[test]
    #[ignore = "a tool, not a check"]
    fn write_keys() {
        let Some(path) = std::env::var_os("I18N_KEYS") else {
            return;
        };
        let keys: Vec<String> = super::tests::asked_for_survey().into_iter().collect();
        std::fs::write(path, serde_json::to_string_pretty(&keys).unwrap()).unwrap();
    }
}
