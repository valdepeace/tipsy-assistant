//! Text helpers for speech: sentence splitting of a streamed reply and cleaning before synthesis.

/// A sentence shorter than this is merged with the following one, so the speech provider is
/// never asked to voice a lonely "¡Ja!".
pub const MIN_SENTENCE_CHARS: usize = 25;

/// Accumulates streamed LLM text and yields complete sentences.
#[derive(Default)]
pub struct SentenceSplitter {
    buf: String,
}

/// A character that ends a sentence for the splitter.
fn is_terminator(c: char) -> bool {
    matches!(c, '.' | '?' | '!' | '…')
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds streamed text and returns every sentence that is now complete.
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            match self.next_cut() {
                Some(cut) => {
                    let sentence = self.buf[..cut].trim().to_string();
                    self.buf = self.buf[cut..].trim_start().to_string();
                    if !sentence.is_empty() {
                        out.push(sentence);
                    }
                }
                None => break,
            }
        }
        out
    }

    /// Byte index just after the first terminator run that is followed by whitespace and closes a
    /// sentence of at least `MIN_SENTENCE_CHARS` characters.
    fn next_cut(&self) -> Option<usize> {
        let chars: Vec<(usize, char)> = self.buf.char_indices().collect();
        let mut i = 0;
        while i < chars.len() {
            if is_terminator(chars[i].1) {
                // Extend over a run of terminators ("?!", "...").
                let mut j = i;
                while j + 1 < chars.len() && is_terminator(chars[j + 1].1) {
                    j += 1;
                }
                let end = chars[j].0 + chars[j].1.len_utf8();
                let followed_by_space = j + 1 < chars.len() && chars[j + 1].1.is_whitespace();
                if followed_by_space && self.buf[..end].trim().chars().count() >= MIN_SENTENCE_CHARS {
                    return Some(end);
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }
        None
    }

    /// Returns whatever is left at the end of the stream.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let rest = rest.trim();
        if rest.is_empty() { None } else { Some(rest.to_string()) }
    }
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF   // pictographs, emoticons, transport, symbols & pictographs ext.
        | 0x2600..=0x27BF   // misc symbols, dingbats
        | 0xFE0F            // variation selector
        | 0x200D            // zero width joiner
    )
}

/// Prepares a sentence for a speech provider.
pub fn clean_for_speech(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev: Option<char> = None;
    for c in text.chars() {
        let drop = match c {
            '`' | '*' | '_' | '#' => true,
            '\'' | '’' => prev.is_some_and(|p| p.is_alphabetic()),
            c if is_emoji(c) => true,
            _ => false,
        };
        if drop {
            // Emojis behave like a space so the words around them do not glue together;
            // Markdown markers and apostrophes simply vanish.
            if is_emoji(c) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
        prev = Some(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A sentence made only of punctuation or symbols is not worth a speech request.
pub fn is_speakable(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split_all(chunks: &[&str]) -> Vec<String> {
        let mut s = SentenceSplitter::new();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(s.push(c));
        }
        out.extend(s.finish());
        out
    }

    #[test]
    fn splits_on_sentence_end_followed_by_space() {
        let out =
            split_all(&["Ozú, quillo, esto es una frase larga. Y esta otra ", "también es bastante larga, compadre."]);
        assert_eq!(
            out,
            vec![
                "Ozú, quillo, esto es una frase larga.".to_string(),
                "Y esta otra también es bastante larga, compadre.".to_string(),
            ]
        );
    }

    #[test]
    fn merges_short_sentences_with_the_next_one() {
        let out = split_all(&["¡Ja! Ozú. Los lunes son un castigo divino, illo. Fin."]);
        assert_eq!(out, vec!["¡Ja! Ozú. Los lunes son un castigo divino, illo.".to_string(), "Fin.".to_string(),]);
    }

    #[test]
    fn handles_question_exclamation_and_ellipsis() {
        let out = split_all(&[
            "¿Tú has visto el pipeline en rojo otra vez? ",
            "Pues mira… ",
            "yo estaba compilando, de verdad te lo digo… y ya.",
        ]);
        assert_eq!(
            out,
            vec![
                "¿Tú has visto el pipeline en rojo otra vez?".to_string(),
                "Pues mira… yo estaba compilando, de verdad te lo digo…".to_string(),
                "y ya.".to_string(),
            ]
        );
    }

    #[test]
    fn does_not_split_without_trailing_whitespace_until_finish() {
        let mut s = SentenceSplitter::new();
        assert!(s.push("Esta frase larga termina en punto pero sin espacio.").is_empty());
        assert_eq!(s.finish(), Some("Esta frase larga termina en punto pero sin espacio.".to_string()));
    }

    #[test]
    fn flushes_remainder_without_punctuation() {
        let out = split_all(&["Un remate sin punto final y ya está"]);
        assert_eq!(out, vec!["Un remate sin punto final y ya está".to_string()]);
    }

    #[test]
    fn finish_on_blank_is_none() {
        let mut s = SentenceSplitter::new();
        s.push("   \n ");
        assert_eq!(s.finish(), None);
    }

    #[test]
    fn decimal_points_and_versions_do_not_split() {
        let out = split_all(&["Actualiza a la versión 2.5.1 del paquete antes de nada, compadre. Venga."]);
        assert_eq!(
            out,
            vec!["Actualiza a la versión 2.5.1 del paquete antes de nada, compadre.".to_string(), "Venga.".to_string(),]
        );
    }

    #[test]
    fn multibyte_chunks_never_panic() {
        // Chunk boundaries fall between multi-byte characters' neighbours.
        let out = split_all(&["ñ", "á", "…", " ¿", "é", "? 🍺🍺 ", "ü."]);
        assert_eq!(out.concat().chars().filter(|c| *c == 'ñ').count(), 1);
    }

    #[test]
    fn cleans_apostrophes_markdown_and_emojis() {
        assert_eq!(clean_for_speech("pa' ná', la verdá"), "pa ná, la verdá");
        assert_eq!(clean_for_speech("pa’l barco"), "pal barco");
        assert_eq!(
            clean_for_speech("usa `git reset --soft HEAD~1` y **listo**"),
            "usa git reset --soft HEAD~1 y listo"
        );
        assert_eq!(clean_for_speech("# Título\n_cursiva_"), "Título cursiva");
        assert_eq!(clean_for_speech("Ozú 🍺🙂  quillo"), "Ozú quillo");
        assert_eq!(clean_for_speech("'entre comillas'"), "'entre comillas");
    }

    #[test]
    fn cleaning_to_nothing_gives_empty_string() {
        assert_eq!(clean_for_speech("🍺🙂 ** `` #"), "");
    }

    #[test]
    fn markdown_markers_leave_no_gap_before_punctuation() {
        assert_eq!(clean_for_speech("Eso es pa' ná', **compadre**, de verdad."), "Eso es pa ná, compadre, de verdad.");
    }

    #[test]
    fn speakable_needs_a_letter_or_digit() {
        assert!(is_speakable("Ozú."));
        assert!(is_speakable("42"));
        assert!(!is_speakable("."));
        assert!(!is_speakable("¿?¡! …"));
        assert!(!is_speakable(""));
    }
}
