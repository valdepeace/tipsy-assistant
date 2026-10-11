//! The login gate's rules (spec `docs/superpowers/specs/2026-10-11-login-screen-design.md`, §5).
//!
//! Pure, synchronous and dependency-free on purpose. The capture that produces an answer's text and
//! the screen that draws the verdict live elsewhere; nothing here knows about a frame, a provider or a
//! socket, which is what lets the whole of what the ritual decides be a table of examples.

use crate::config::model::{LoginConfig, LoginEntry};

/// The ruling on one answer: whether the door opens, and who it opened for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub ok: bool,
    /// The matched entry's `name`, or `None` when that entry has none — the screen then greets without
    /// a name (`Pasa.`).
    pub name: Option<String>,
}

impl Verdict {
    /// The ruling when there is no screen to answer (spec §4.1): nothing to check, nobody refused.
    pub fn open() -> Self {
        Self { ok: true, name: None }
    }

    /// The ruling on an answer none of the keywords matched.
    pub fn refused() -> Self {
        Self { ok: false, name: None }
    }
}

/// True when the login screen applies at all (R1): the section is on **and** at least one entry has a
/// keyword that survives normalization. A mis-edited file can therefore never slam the door: no usable
/// keyword, no screen.
pub fn applies(login: &LoginConfig) -> bool {
    login.enabled && login.allowed.iter().any(|entry| !usable_keywords(entry).is_empty())
}

/// The tokens of one answer or keyword (R2): lowercased, Latin diacritics folded, split on everything
/// that is not alphanumeric (Unicode: `ñ` and `á` are letters, `¿` is not).
pub fn tokens(s: &str) -> Vec<String> {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(fold)
        .collect::<String>()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The first entry, in file order, whose every keyword is in the answer (R3–R6).
pub fn judge(login: &LoginConfig, answer: &str) -> Verdict {
    if !applies(login) {
        return Verdict::open();
    }
    let said = tokens(answer);
    login
        .allowed
        .iter()
        .find(|entry| {
            let keywords = usable_keywords(entry);
            !keywords.is_empty() && keywords.iter().all(|keyword| contains_run(&said, keyword))
        })
        .map(|entry| Verdict {
            ok: true,
            name: (!entry.name.trim().is_empty()).then(|| entry.name.clone()),
        })
        .unwrap_or_else(Verdict::refused)
}

/// An entry's keywords, normalized, with the ones that normalize to nothing dropped: an empty keyword
/// is ignored rather than matching everything (R6).
fn usable_keywords(entry: &LoginEntry) -> Vec<Vec<String>> {
    entry.keywords.iter().map(|keyword| tokens(keyword)).filter(|keyword| !keyword.is_empty()).collect()
}

/// True when `keyword` appears as a contiguous run of `said`'s tokens (R3): `score` is not `core`, and
/// a multi-word keyword has to be said in one go.
fn contains_run(said: &[String], keyword: &[String]) -> bool {
    !keyword.is_empty() && keyword.len() <= said.len() && said.windows(keyword.len()).any(|w| w == keyword)
}

/// The Spanish and Galician letters this project's profiles write, folded to their plain form. A
/// character with no entry keeps its own lowercase form, so a profile in another language still works.
fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ý' | 'ÿ' => 'y',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::LoginEntry;

    fn entry(name: &str, keywords: &[&str]) -> LoginEntry {
        LoginEntry { name: name.into(), keywords: keywords.iter().map(|k| (*k).into()).collect() }
    }

    fn login(enabled: bool, entries: Vec<LoginEntry>) -> LoginConfig {
        LoginConfig { enabled, voice: true, allowed: entries }
    }

    /// R2, and the reason `char::is_alphanumeric` is the predicate: a naive `is_ascii_alphanumeric`
    /// splits `á` off its word, and `ñ` is a letter in this project's profiles.
    #[test]
    fn tokens_fold_case_accents_and_punctuation() {
        assert_eq!(tokens("ANDRÉS, del CORE!!"), vec!["andres", "del", "core"]);
        assert_eq!(tokens("el niño del core"), vec!["el", "nino", "del", "core"]);
        assert_eq!(tokens("MARÍA-JOSÉ  y   el  core"), vec!["maria", "jose", "y", "el", "core"]);
        assert_eq!(tokens("  "), Vec::<String>::new());
    }

    /// Review Focus 2: an answer that is only punctuation, emoji or spaces tokenizes to nothing, and
    /// an empty keyword must never match it by accident.
    #[test]
    fn an_answer_of_punctuation_or_emoji_alone_is_judged_and_refused() {
        let config = login(true, vec![entry("Andrés", &["andres", "core"])]);
        for answer in ["", "   ", "!!!", "¿?", "🔥", "«»"] {
            assert_eq!(judge(&config, answer), Verdict::refused(), "{answer:?}");
        }
        // An entry whose keywords are all punctuation is not a usable entry either: R1 says the
        // screen does not even appear, and R6 says it never matches.
        let blank = login(true, vec![entry("Nadie", &["", "  ", "!!!"])]);
        assert!(!applies(&blank), "blank keywords are not a usable allowlist");
        assert_eq!(judge(&blank, "!!!"), Verdict::open(), "…and with no screen there is nothing to refuse");
    }

    /// R1: both halves are needed, and the second one is the one that keeps an owner from locking
    /// themselves out of their own application.
    #[test]
    fn applies_needs_the_switch_and_one_usable_keyword() {
        assert!(applies(&login(true, vec![entry("Andrés", &["andres"])])));
        assert!(applies(&login(true, vec![entry("", &["andres"])])), "a name is not required");
        assert!(!applies(&login(false, vec![entry("Andrés", &["andres"])])), "the switch is the switch");
        assert!(!applies(&login(true, Vec::new())), "no entries, no screen");
        assert!(!applies(&login(true, vec![entry("Andrés", &[])])), "no keywords, no screen");
        assert!(!applies(&login(true, vec![entry("Andrés", &["   ", ""])])), "blank is not a keyword");
    }

    /// The spec's own worked examples (§5.2), driven as a table so the wording and the code cannot
    /// drift apart.
    #[test]
    fn the_worked_examples_of_the_spec() {
        let config = login(true, vec![entry("Andrés", &["andres", "core"]), entry("María", &["maria", "data"])]);
        let cases: &[(&str, Verdict)] = &[
            ("yo soy Andrés del equipo del core", Verdict { ok: true, name: Some("Andrés".into()) }),
            ("ANDRÉS, del CORE!!", Verdict { ok: true, name: Some("Andrés".into()) }),
            ("soy andres del score", Verdict::refused()),
            ("soy Andrés", Verdict::refused()),
            ("soy María, conozco a Andrés del core", Verdict { ok: true, name: Some("Andrés".into()) }),
            ("soy Maria de data", Verdict { ok: true, name: Some("María".into()) }),
            ("el niño del core", Verdict::refused()),
        ];
        for (answer, expected) in cases {
            assert_eq!(&judge(&config, answer), expected, "{answer:?}");
        }
        let nino = login(true, vec![entry("El Niño", &["nino", "core"])]);
        assert_eq!(judge(&nino, "el niño del core"), Verdict { ok: true, name: Some("El Niño".into()) });
    }

    /// R3: whole tokens, so a keyword is never a substring of a longer word.
    #[test]
    fn a_keyword_is_never_a_substring() {
        let score = login(true, vec![entry("Andrés", &["andres", "core"])]);
        assert_eq!(judge(&score, "soy andres del score"), Verdict::refused());
        let short = login(true, vec![entry("Andrés", &["andres"])]);
        assert_eq!(judge(&short, "soy andresito del core"), Verdict::refused(), "andresito is not andres");
    }

    /// R3's other half: a multi-word keyword is one run, said together.
    #[test]
    fn a_multi_word_keyword_must_be_said_in_one_run() {
        let config = login(true, vec![entry("Andrés", &["andres del equipo", "core"])]);
        assert_eq!(judge(&config, "soy andres del equipo del core"), Verdict { ok: true, name: Some("Andrés".into()) });
        assert_eq!(judge(&config, "soy andres, y el equipo del core mola"), Verdict::refused());
    }

    /// R5: file order is the ranking, and the first match wins even when a later entry would match too.
    /// The answer carries every keyword of *both* entries on purpose: R4 makes an entry match only when
    /// all of its keywords are said, so the plan's shorter `"andres y maria"` matched the second entry
    /// alone and exercised no ranking at all.
    #[test]
    fn the_first_entry_in_file_order_wins() {
        let config = login(true, vec![entry("Andrés", &["andres", "core"]), entry("María", &["maria", "andres"])]);
        assert_eq!(judge(&config, "soy andres del core y maria"), Verdict { ok: true, name: Some("Andrés".into()) });
    }

    /// R6: a keyword written the way a person writes it behaves like its folded form, and an empty
    /// name is a `None` rather than an empty greeting.
    #[test]
    fn keywords_are_normalized_as_written_and_an_empty_name_is_none() {
        let written = login(true, vec![entry("Andrés", &["Andrés", "CORE"])]);
        assert_eq!(judge(&written, "soy andres del core"), Verdict { ok: true, name: Some("Andrés".into()) });
        let nameless = login(true, vec![entry("  ", &["andres"])]);
        assert_eq!(judge(&nameless, "andres"), Verdict::open(), "an empty name opens without one");
    }
}
