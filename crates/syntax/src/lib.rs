//! LaTeX source understanding for TexCraft: syntax colouring, document outline, magic
//! comments, autocompletion, bibliography keys and word counts.
//!
//! Pure functions over `&str`; no I/O, compiles for wasm32. Never panics on any input.

pub mod bib;
pub mod completion;
pub mod lex;
pub mod magic;
pub mod outline;
pub mod words;

pub use bib::{BibEntry, bib_entries};
pub use completion::{CompletionContext, context_at};
pub use lex::{Span, TokenKind, highlight};
pub use magic::{MagicComments, is_root_document, magic_comments};
pub use outline::{OutlineItem, OutlineKind, defined_commands, labels, outline};
pub use words::word_count;

/// Byte offset of the bracket matching the one at `pos` (`{}`, `[]`, `()`), skipping escaped
/// brackets. `None` if `pos` is not a bracket or it is unmatched.
pub fn matching_bracket(text: &str, pos: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let c = *bytes.get(pos)?;
    let escaped = |i: usize| {
        let mut n = 0usize;
        let mut j = i;
        while j > 0 && bytes.get(j - 1) == Some(&b'\\') {
            n += 1;
            j -= 1;
        }
        n % 2 == 1
    };
    if escaped(pos) {
        return None;
    }
    let (open, close, forward) = match c {
        b'{' => (b'{', b'}', true),
        b'[' => (b'[', b']', true),
        b'(' => (b'(', b')', true),
        b'}' => (b'{', b'}', false),
        b']' => (b'[', b']', false),
        b')' => (b'(', b')', false),
        _ => return None,
    };
    let mut depth = 0i64;
    if forward {
        let mut i = pos;
        while let Some(&b) = bytes.get(i) {
            if !escaped(i) {
                if b == open {
                    depth += 1;
                } else if b == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
            }
            i += 1;
        }
    } else {
        let mut i = pos;
        loop {
            let b = *bytes.get(i)?;
            if !escaped(i) {
                if b == close {
                    depth += 1;
                } else if b == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
            }
            if i == 0 {
                return None;
            }
            i -= 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets() {
        let t = "\\frac{a}{b \\{ c}[x]";
        assert_eq!(matching_bracket(t, 5), Some(7));
        assert_eq!(matching_bracket(t, 8), Some(15));
        assert_eq!(matching_bracket(t, 15), Some(8));
        assert_eq!(matching_bracket(t, 12), None); // escaped
        assert_eq!(matching_bracket("{", 0), None);
        assert_eq!(matching_bracket("}", 0), None);
    }
}
