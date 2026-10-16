//! Word counting that ignores markup.

use crate::lex::{TokenKind, highlight};

/// Counts the words a reader would see: text outside commands, comments, math and verbatim.
/// Each CJK character counts as one word, the convention for Chinese and Japanese.
pub fn word_count(text: &str) -> usize {
    // Skip the preamble when there is a document body.
    let body_start = text.find("\\begin{document}").unwrap_or(0);
    let body = text.get(body_start..).unwrap_or(text);
    let mut count = 0usize;
    for span in highlight(body) {
        if span.kind != TokenKind::Text {
            continue;
        }
        let Some(s) = body.get(span.range.clone()) else {
            continue;
        };
        let mut in_word = false;
        for c in s.chars() {
            if is_cjk(c) {
                count += 1;
                in_word = false;
            } else if c.is_alphanumeric() || c == '\'' || c == '-' {
                if !in_word && c.is_alphanumeric() {
                    count += 1;
                    in_word = true;
                }
            } else {
                in_word = false;
            }
        }
    }
    count
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0x20000..=0x2FA1F)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_visible_words_only() {
        let doc = "\\documentclass{article}\n\\usepackage{amsmath}\n\\begin{document}\n\\section{Intro}\nHello \\emph{brave} new-world, it's $x^2$ here. % not counted\n\\end{document}";
        // Intro, Hello, brave, new-world, it's, here
        assert_eq!(word_count(doc), 6);
    }

    #[test]
    fn counts_cjk_characters() {
        assert_eq!(word_count("中文 text"), 3);
    }
}
