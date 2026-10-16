//! Pure text-editing operations used by commands (toggle comment, wrap selection, indent).
//!
//! Positions are **char** indices (what the editor widget uses); helpers convert to byte offsets.

/// A selection in char indices. `anchor` is where it started, `cursor` where it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    /// Fixed end.
    pub anchor: usize,
    /// Moving end (the caret).
    pub cursor: usize,
}

impl Selection {
    /// A caret at `at`.
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            cursor: at,
        }
    }

    /// Smaller end.
    pub fn start(self) -> usize {
        self.anchor.min(self.cursor)
    }

    /// Larger end.
    pub fn end(self) -> usize {
        self.anchor.max(self.cursor)
    }

    /// Nothing selected.
    pub fn is_empty(self) -> bool {
        self.anchor == self.cursor
    }
}

/// Byte offset of char index `ci` in `text` (clamped to the end).
pub fn byte_of(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b)
}

/// Char index of byte offset `b` (clamped; a non-boundary rounds down).
pub fn char_of(text: &str, b: usize) -> usize {
    let mut b = b.min(text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    text.get(..b).map_or(0, |s| s.chars().count())
}

/// 1-based line of char index `ci`.
pub fn line_of_char(text: &str, ci: usize) -> u32 {
    let b = byte_of(text, ci);
    let n = text
        .get(..b)
        .map_or(0, |s| s.bytes().filter(|&c| c == b'\n').count());
    u32::try_from(n + 1).unwrap_or(u32::MAX)
}

/// 1-based column (in chars) of char index `ci`.
pub fn column_of_char(text: &str, ci: usize) -> u32 {
    let b = byte_of(text, ci);
    let line_start = text
        .get(..b)
        .and_then(|s| s.rfind('\n'))
        .map_or(0, |i| i + 1);
    let n = text.get(line_start..b).map_or(0, |s| s.chars().count());
    u32::try_from(n + 1).unwrap_or(u32::MAX)
}

/// Char index of the start of 1-based `line` (clamped to the last line).
pub fn char_of_line(text: &str, line: u32) -> usize {
    let mut remaining = line.saturating_sub(1);
    if remaining == 0 {
        return 0;
    }
    for (ci, c) in text.chars().enumerate() {
        if c == '\n' {
            remaining -= 1;
            if remaining == 0 {
                return ci + 1;
            }
        }
    }
    // Past the end: start of the last line.
    let last_nl = text.rfind('\n').map_or(0, |i| i + 1);
    char_of(text, last_nl)
}

/// Byte range of the full lines touched by byte range `start..end`.
fn line_span(text: &str, start: usize, end: usize) -> (usize, usize) {
    let ls = text
        .get(..start)
        .and_then(|s| s.rfind('\n'))
        .map_or(0, |i| i + 1);
    // A selection ending right after a newline does not include the next line.
    let end_eff = if end > start && text.as_bytes().get(end - 1) == Some(&b'\n') {
        end - 1
    } else {
        end
    };
    let le = text
        .get(end_eff..)
        .and_then(|s| s.find('\n'))
        .map_or(text.len(), |i| end_eff + i);
    (ls, le.max(ls))
}

/// Applies `f` to every line touched by `sel`, keeping the selection on the same lines.
fn map_lines(
    text: &str,
    sel: Selection,
    f: impl Fn(&[&str]) -> Vec<String>,
) -> (String, Selection) {
    let (s, e) = (byte_of(text, sel.start()), byte_of(text, sel.end()));
    let (ls, le) = line_span(text, s, e);
    let block = text.get(ls..le).unwrap_or("");
    let lines: Vec<&str> = block.split('\n').collect();
    let new_block = f(&lines).join("\n");
    let mut out = String::with_capacity(text.len() + new_block.len());
    out.push_str(text.get(..ls).unwrap_or(""));
    out.push_str(&new_block);
    out.push_str(text.get(le..).unwrap_or(""));
    let start_c = char_of(&out, ls);
    let end_c = start_c + new_block.chars().count();
    let new_sel = if sel.is_empty() {
        // Keep the caret on its line, shifted by the change in this line's prefix.
        let old_line_len = lines.first().map_or(0, |l| l.chars().count());
        let new_line_len = new_block
            .split('\n')
            .next()
            .map_or(0, |l| l.chars().count());
        let col = sel.cursor.saturating_sub(char_of(text, ls));
        let shifted = (col as i64 + new_line_len as i64 - old_line_len as i64).max(0) as usize;
        Selection::caret(start_c + shifted.min(new_line_len))
    } else {
        Selection {
            anchor: start_c,
            cursor: end_c,
        }
    };
    (out, new_sel)
}

/// Comments the touched lines with `% `, or uncomments them if all are commented.
pub fn toggle_comment(text: &str, sel: Selection) -> (String, Selection) {
    map_lines(text, sel, |lines| {
        let all_commented = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .all(|l| l.trim_start().starts_with('%'));
        let indent = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);
        lines
            .iter()
            .map(|l| {
                if l.trim().is_empty() {
                    (*l).to_string()
                } else if all_commented {
                    let lead = l.len() - l.trim_start().len();
                    let rest = l.get(lead..).unwrap_or("");
                    let rest = rest
                        .strip_prefix("% ")
                        .or_else(|| rest.strip_prefix('%'))
                        .unwrap_or(rest);
                    format!("{}{}", l.get(..lead).unwrap_or(""), rest)
                } else {
                    let cut = indent.min(l.len());
                    format!(
                        "{}% {}",
                        l.get(..cut).unwrap_or(""),
                        l.get(cut..).unwrap_or("")
                    )
                }
            })
            .collect()
    })
}

/// Indents the touched lines by two spaces.
pub fn indent(text: &str, sel: Selection) -> (String, Selection) {
    map_lines(text, sel, |lines| {
        lines.iter().map(|l| format!("  {l}")).collect()
    })
}

/// Removes up to two leading spaces (or one tab) from the touched lines.
pub fn outdent(text: &str, sel: Selection) -> (String, Selection) {
    map_lines(text, sel, |lines| {
        lines
            .iter()
            .map(|l| {
                l.strip_prefix("  ")
                    .or_else(|| l.strip_prefix('\t'))
                    .or_else(|| l.strip_prefix(' '))
                    .unwrap_or(l)
                    .to_string()
            })
            .collect()
    })
}

/// Wraps the selection in `before`/`after` (e.g. `\textbf{` / `}`). With an empty selection,
/// inserts both and puts the caret between them. If the selection is already wrapped exactly,
/// unwraps it.
pub fn wrap(text: &str, sel: Selection, before: &str, after: &str) -> (String, Selection) {
    let (s, e) = (byte_of(text, sel.start()), byte_of(text, sel.end()));
    let inner = text.get(s..e).unwrap_or("");
    // Unwrap if the text around the selection is exactly before/after.
    if !inner.is_empty()
        && s >= before.len()
        && text.get(s - before.len()..s) == Some(before)
        && text.get(e..e + after.len()) == Some(after)
    {
        let mut out = String::with_capacity(text.len());
        out.push_str(text.get(..s - before.len()).unwrap_or(""));
        out.push_str(inner);
        out.push_str(text.get(e + after.len()..).unwrap_or(""));
        let a = sel.start() - before.chars().count();
        return (
            out,
            Selection {
                anchor: a,
                cursor: a + inner.chars().count(),
            },
        );
    }
    let mut out = String::with_capacity(text.len() + before.len() + after.len());
    out.push_str(text.get(..s).unwrap_or(""));
    out.push_str(before);
    out.push_str(inner);
    out.push_str(after);
    out.push_str(text.get(e..).unwrap_or(""));
    let a = sel.start() + before.chars().count();
    let new_sel = if inner.is_empty() {
        Selection::caret(a)
    } else {
        Selection {
            anchor: a,
            cursor: a + inner.chars().count(),
        }
    };
    (out, new_sel)
}

/// Replaces the selection with `insert` (which may contain `$1`/`$0` snippet markers) and
/// places the caret at the snippet cursor; the selected text, if any, goes where `$SEL` is.
pub fn insert_snippet(text: &str, sel: Selection, snippet: &str) -> (String, Selection) {
    let (s, e) = (byte_of(text, sel.start()), byte_of(text, sel.end()));
    let selected = text.get(s..e).unwrap_or("");
    // Indent continuation lines like the current line.
    let line_start = text
        .get(..s)
        .and_then(|t| t.rfind('\n'))
        .map_or(0, |i| i + 1);
    let indent: String = text
        .get(line_start..s)
        .unwrap_or("")
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    let with_sel = snippet.replace("$SEL", selected);
    let indented = with_sel.replace('\n', &format!("\n{indent}"));
    let (body, cursor_b) = texcraft_syntax::completion::expand_snippet(&indented);
    let mut out = String::with_capacity(text.len() + body.len());
    out.push_str(text.get(..s).unwrap_or(""));
    out.push_str(&body);
    out.push_str(text.get(e..).unwrap_or(""));
    let caret = sel.start() + char_of(&body, cursor_b);
    (out, Selection::caret(caret))
}

/// Replaces the selection with `insert` literally; the caret ends after it.
pub fn replace_selection(text: &str, sel: Selection, insert: &str) -> (String, Selection) {
    let (s, e) = (byte_of(text, sel.start()), byte_of(text, sel.end()));
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(text.get(..s).unwrap_or(""));
    out.push_str(insert);
    out.push_str(text.get(e..).unwrap_or(""));
    (out, Selection::caret(sel.start() + insert.chars().count()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        let t = "ab\n中文x\n";
        assert_eq!(byte_of(t, 3), 3);
        assert_eq!(byte_of(t, 4), 6);
        assert_eq!(char_of(t, 6), 4);
        assert_eq!(char_of(t, 5), 3); // inside 中 rounds down
        assert_eq!(line_of_char(t, 5), 2);
        assert_eq!(column_of_char(t, 5), 3);
        assert_eq!(char_of_line(t, 2), 3);
        assert_eq!(char_of_line(t, 99), 7);
        assert_eq!(char_of_line(t, 0), 0);
    }

    #[test]
    fn comment_toggle_round_trips() {
        let t = "a\n  b\n\n  c\nd";
        let sel = Selection {
            anchor: 2,
            cursor: 9,
        }; // lines 2..4
        let (c, s) = toggle_comment(t, sel);
        assert_eq!(c, "a\n  % b\n\n  % c\nd");
        let (u, _) = toggle_comment(&c, s);
        assert_eq!(u, t);
        // Caret only: comments the current line and keeps the caret after the same char.
        let (c2, s2) = toggle_comment("xy", Selection::caret(1));
        assert_eq!(c2, "% xy");
        assert_eq!(s2, Selection::caret(3));
    }

    #[test]
    fn indent_outdent() {
        let (i, s) = indent(
            "a\nb",
            Selection {
                anchor: 0,
                cursor: 3,
            },
        );
        assert_eq!(i, "  a\n  b");
        let (o, _) = outdent(&i, s);
        assert_eq!(o, "a\nb");
    }

    #[test]
    fn wrap_and_unwrap() {
        let (w, s) = wrap(
            "say hi",
            Selection {
                anchor: 4,
                cursor: 6,
            },
            "\\textbf{",
            "}",
        );
        assert_eq!(w, "say \\textbf{hi}");
        assert_eq!(
            s,
            Selection {
                anchor: 12,
                cursor: 14
            }
        );
        let (u, _) = wrap(&w, s, "\\textbf{", "}");
        assert_eq!(u, "say hi");
        let (e, s) = wrap("x", Selection::caret(1), "$", "$");
        assert_eq!((e.as_str(), s), ("x$$", Selection::caret(2)));
    }

    #[test]
    fn snippets_indent_and_take_selection() {
        let (t, s) = insert_snippet(
            "  ab",
            Selection {
                anchor: 2,
                cursor: 4,
            },
            "\\begin{center}\n  $SEL$0\n\\end{center}",
        );
        assert_eq!(t, "  \\begin{center}\n    ab\n  \\end{center}");
        assert_eq!(
            &t.chars().take(s.cursor).collect::<String>(),
            "  \\begin{center}\n    ab"
        );
    }

    #[test]
    fn out_of_range_selections_do_not_panic() {
        let t = "中";
        let sel = Selection {
            anchor: 9,
            cursor: 99,
        };
        let _ = toggle_comment(t, sel);
        let _ = wrap(t, sel, "{", "}");
        let _ = indent(t, sel);
        let _ = insert_snippet(t, sel, "x$0");
    }
}
