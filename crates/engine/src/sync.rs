//! Where inverse search lands in a source line, and what it highlights.
//!
//! SyncTeX names a line (engines almost never record a column). The PDF viewer also knows the
//! word under the click and the text around it; finding that in the line puts the caret on the
//! clicked character and narrows the highlight to its sentence, as the target app does. Without
//! a match the caret goes to the start of the line and the whole line flashes.

use std::ops::Range;

/// The text around an inverse-search click, from the PDF's text layer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncHint {
    /// Text just before the word (same PDF line).
    pub before: String,
    /// The word (or CJK character) clicked.
    pub word: String,
    /// Text just after it.
    pub after: String,
    /// Characters into `word` where the click fell.
    pub offset: usize,
}

/// Characters that end a sentence. A full stop counts only before white space or the line end
/// (not in `3.14`).
fn ends_sentence(chars: &[char], i: usize) -> bool {
    match chars.get(i) {
        Some('。' | '！' | '？' | '!' | '?') => true,
        Some('.') => chars.get(i + 1).is_none_or(|c| c.is_whitespace()),
        _ => false,
    }
}

/// All char offsets where `needle` occurs in `hay`.
fn occurrences(hay: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    hay.windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(i, _)| i)
        .collect()
}

/// Finds the hint in the line: the clicked character's offset. Tries the word with less and
/// less context around it; the first unique match wins, else the first match with the most
/// context.
fn find_hint(chars: &[char], hint: &SyncHint) -> Option<usize> {
    let word: Vec<char> = hint.word.chars().collect();
    if word.is_empty() {
        return None;
    }
    let before: Vec<char> = hint.before.chars().collect();
    let after: Vec<char> = hint.after.chars().collect();
    let offset = hint.offset.min(word.len().saturating_sub(1));
    let mut fallback = None;
    for k in [8usize, 4, 2, 1, 0] {
        let b = before
            .get(before.len().saturating_sub(k)..)
            .unwrap_or_default();
        let a = after.get(..k.min(after.len())).unwrap_or_default();
        let needle: Vec<char> = b.iter().chain(&word).chain(a).copied().collect();
        let found = occurrences(chars, &needle);
        let at = |start: usize| start + b.len() + offset;
        match found.as_slice() {
            [one] => return Some(at(*one)),
            [first, ..] if fallback.is_none() => fallback = Some(at(*first)),
            _ => {}
        }
    }
    fallback
}

/// The sentence around char `at` of the line, within `content` (the line without its
/// surrounding white space).
fn sentence(chars: &[char], at: usize, content: Range<usize>) -> Range<usize> {
    let mut start = content.start;
    let mut i = at.min(content.end);
    while i > content.start {
        if ends_sentence(chars, i - 1) && i - 1 < at {
            start = i;
            break;
        }
        i -= 1;
    }
    while start < content.end && chars.get(start).is_some_and(|c| c.is_whitespace()) {
        start += 1;
    }
    let mut end = content.end;
    let mut j = at.max(start);
    while j < content.end {
        if ends_sentence(chars, j) {
            end = j + 1;
            break;
        }
        j += 1;
    }
    if start >= end {
        return content;
    }
    start..end
}

/// For source `line` (its text, without the line break): the caret offset and the range to
/// highlight, both in characters from the start of the line. `column` is SyncTeX's (1-based).
pub fn locate(line: &str, column: Option<u32>, hint: Option<&SyncHint>) -> (usize, Range<usize>) {
    let chars: Vec<char> = line.chars().collect();
    let lead = chars.iter().take_while(|c| c.is_whitespace()).count();
    let tail = chars.len() - chars.iter().rev().take_while(|c| c.is_whitespace()).count();
    let content = lead..tail.max(lead);
    let anchor = hint.and_then(|h| find_hint(&chars, h)).or_else(|| {
        column
            .and_then(|c| usize::try_from(c).ok())
            .and_then(|c| c.checked_sub(1))
            .filter(|&c| c > 0 && c < chars.len())
    });
    match anchor {
        Some(at) => (at, sentence(&chars, at, content)),
        None => (lead, content),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(before: &str, word: &str, after: &str, offset: usize) -> SyncHint {
        SyncHint {
            before: before.into(),
            word: word.into(),
            after: after.into(),
            offset,
        }
    }

    fn flashed(line: &str, r: Range<usize>) -> String {
        line.chars().skip(r.start).take(r.len()).collect()
    }

    #[test]
    fn whole_line_without_a_hint() {
        let line = "  Start writing here. Equations are numbered:  ";
        let (caret, r) = locate(line, None, None);
        assert_eq!(caret, 2);
        assert_eq!(
            flashed(line, r),
            "Start writing here. Equations are numbered:"
        );
        assert_eq!(locate("", None, None), (0, 0..0));
        assert_eq!(locate("   ", None, None), (3, 3..3));
    }

    #[test]
    fn a_hint_narrows_to_its_sentence() {
        let line = "Start writing here. Equations are numbered automatically: see e.g. 3.14 too";
        let (caret, r) = locate(line, None, Some(&hint("", "Equations", "are", 2)));
        assert_eq!(caret, line.find("Equations").unwrap() + 2);
        assert_eq!(
            flashed(line, r),
            "Equations are numbered automatically: see e.g."
        );
        // The first sentence, with its full stop.
        let (_, r) = locate(line, None, Some(&hint("Start", "writing", "", 0)));
        assert_eq!(flashed(line, r), "Start writing here.");
        // No match: the whole line.
        let (caret, r) = locate(line, None, Some(&hint("", "absent", "", 0)));
        assert_eq!((caret, r), (0, 0..line.chars().count()));
    }

    #[test]
    fn context_picks_among_repeats() {
        // The user's test paragraph repeats the same words; context decides.
        let line = "测试实时编译预览测试实时编译预览。你好你好测试实时编译预览";
        let (caret, r) = locate(line, None, Some(&hint("你好", "测", "试实时", 0)));
        assert_eq!(caret, 21);
        assert_eq!(flashed(line, r), "你好你好测试实时编译预览");
        // Ambiguous even with context: the first occurrence.
        let (caret, r) = locate(line, None, Some(&hint("", "实", "时", 0)));
        assert_eq!(caret, 2);
        assert_eq!(flashed(line, r), "测试实时编译预览测试实时编译预览。");
        // Source markup around the word: the PDF context does not match, the word does.
        let line = r"Refer back to Section~\ref{sec:method}.";
        let (caret, _) = locate(line, None, Some(&hint("back to", "Section", " 2.", 3)));
        assert_eq!(caret, line.find("Section").unwrap() + 3);
    }

    #[test]
    fn column_when_recorded() {
        let line = "One. Two three. Four.";
        let (caret, r) = locate(line, Some(7), None);
        assert_eq!(caret, 6);
        assert_eq!(flashed(line, r), "Two three.");
        // Column 1 (or past the end) says nothing.
        assert_eq!(locate(line, Some(1), None).0, 0);
        assert_eq!(locate(line, Some(99), None).1, 0..21);
    }
}
