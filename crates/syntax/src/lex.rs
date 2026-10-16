//! A small, fast LaTeX lexer for syntax colouring.
//!
//! It is not a TeX parser: it recognises what an editor colours (commands, comments, math,
//! environments, reference keys, verbatim) and never fails. The output spans cover the whole
//! input, in order, without gaps.

use std::ops::Range;

/// What a span of source is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Plain text.
    Text,
    /// A control sequence (`\foo`, `\\`, `\%`).
    Command,
    /// A sectioning command (`\section`, `\chapter`, ...): drawn with more weight.
    Sectioning,
    /// `\begin` / `\end`.
    EnvironmentCommand,
    /// The environment name in `\begin{name}` / `\end{name}`.
    EnvironmentName,
    /// The key in `\label{..}`, `\ref{..}`, `\cite{..}` and friends.
    Reference,
    /// `{` or `}`.
    Brace,
    /// `[` or `]` (optional-argument brackets).
    Bracket,
    /// `$`, `$$`, `\(`, `\)`, `\[`, `\]`.
    MathDelimiter,
    /// Text inside math.
    Math,
    /// A command inside math.
    MathCommand,
    /// `&`, `~`, `^`, `_`, `#`.
    Special,
    /// `% ...` to the end of the line.
    Comment,
    /// `% !TEX ...` magic comment.
    MagicComment,
    /// Verbatim content (`\verb|..|`, `verbatim`, `lstlisting`, `minted`, `comment` bodies).
    Verbatim,
}

/// A coloured byte range of the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Byte range into the source text (always on char boundaries).
    pub range: Range<usize>,
    /// What it is.
    pub kind: TokenKind,
}

const SECTIONING: &[&str] = &[
    "part",
    "chapter",
    "section",
    "subsection",
    "subsubsection",
    "paragraph",
    "subparagraph",
    "frametitle",
];

/// Commands whose first braced argument is a label/citation key (coloured as [`TokenKind::Reference`]).
pub const REFERENCE_COMMANDS: &[&str] = &[
    "label",
    "ref",
    "eqref",
    "pageref",
    "autoref",
    "nameref",
    "cref",
    "Cref",
    "vref",
    "cite",
    "citep",
    "citet",
    "citealp",
    "citealt",
    "citeauthor",
    "citeyear",
    "parencite",
    "textcite",
    "autocite",
    "footcite",
    "nocite",
];

/// Environments whose content is shown as math.
pub const MATH_ENVIRONMENTS: &[&str] = &[
    "equation",
    "equation*",
    "align",
    "align*",
    "alignat",
    "alignat*",
    "flalign",
    "flalign*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "eqnarray",
    "eqnarray*",
    "math",
    "displaymath",
];

/// Environments whose content is verbatim.
pub const VERBATIM_ENVIRONMENTS: &[&str] = &[
    "verbatim",
    "verbatim*",
    "Verbatim",
    "lstlisting",
    "minted",
    "comment",
    "filecontents",
    "filecontents*",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum MathMode {
    None,
    /// `$ ... $`
    Inline,
    /// `$$ ... $$`
    Display,
    /// `\( ... \)` or `\[ ... \]`: closed by the matching command.
    Paren,
    Bracket,
    /// A math environment: closed by `\end{...}`.
    Env,
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    spans: Vec<Span>,
    math: MathMode,
    math_env_depth: u32,
}

/// Colours `text`. The returned spans are contiguous and cover all of `text`.
pub fn highlight(text: &str) -> Vec<Span> {
    let mut lx = Lexer {
        src: text,
        bytes: text.as_bytes(),
        pos: 0,
        spans: Vec::with_capacity(text.len() / 4 + 1),
        math: MathMode::None,
        math_env_depth: 0,
    };
    lx.run();
    lx.spans
}

impl<'a> Lexer<'a> {
    fn push(&mut self, start: usize, end: usize, kind: TokenKind) {
        if end <= start {
            return;
        }
        if let Some(last) = self.spans.last_mut()
            && last.kind == kind
            && last.range.end == start
        {
            last.range.end = end;
            return;
        }
        self.spans.push(Span {
            range: start..end,
            kind,
        });
    }

    fn peek(&self, off: usize) -> Option<u8> {
        self.bytes.get(self.pos.saturating_add(off)).copied()
    }

    fn text_kind(&self) -> TokenKind {
        if self.math == MathMode::None {
            TokenKind::Text
        } else {
            TokenKind::Math
        }
    }

    /// Advances over one UTF-8 character, returning its byte length.
    fn char_len_at(&self, pos: usize) -> usize {
        self.src
            .get(pos..)
            .and_then(|s| s.chars().next())
            .map_or(1, char::len_utf8)
    }

    fn run(&mut self) {
        while self.pos < self.bytes.len() {
            let start = self.pos;
            let Some(b) = self.peek(0) else { break };
            match b {
                b'%' => self.comment(),
                b'\\' => self.command(),
                b'$' => self.dollar(),
                b'{' | b'}' => {
                    self.pos += 1;
                    self.push(start, self.pos, TokenKind::Brace);
                }
                b'[' | b']' if self.math == MathMode::None => {
                    self.pos += 1;
                    self.push(start, self.pos, TokenKind::Bracket);
                }
                b'&' | b'~' | b'^' | b'_' | b'#' => {
                    self.pos += 1;
                    let kind = if self.math == MathMode::None {
                        TokenKind::Special
                    } else {
                        TokenKind::Math
                    };
                    self.push(start, self.pos, kind);
                }
                _ => {
                    // Plain run up to the next interesting byte.
                    let mut end = self.pos;
                    while let Some(&c) = self.bytes.get(end) {
                        if matches!(
                            c,
                            b'%' | b'\\'
                                | b'$'
                                | b'{'
                                | b'}'
                                | b'['
                                | b']'
                                | b'&'
                                | b'~'
                                | b'^'
                                | b'_'
                                | b'#'
                        ) {
                            break;
                        }
                        end += self.char_len_at(end);
                    }
                    if end == self.pos {
                        end += self.char_len_at(end);
                    }
                    self.pos = end.min(self.bytes.len());
                    let kind = self.text_kind();
                    self.push(start, self.pos, kind);
                }
            }
        }
    }

    fn line_end(&self, from: usize) -> usize {
        self.bytes
            .get(from..)
            .and_then(|rest| rest.iter().position(|&c| c == b'\n'))
            .map_or(self.bytes.len(), |i| from + i)
    }

    fn comment(&mut self) {
        let start = self.pos;
        let end = self.line_end(start);
        let body = self.src.get(start..end).unwrap_or("");
        let trimmed = body.trim_start_matches('%').trim_start();
        let kind = if trimmed.len() >= 4
            && trimmed
                .get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case("!tex"))
        {
            TokenKind::MagicComment
        } else {
            TokenKind::Comment
        };
        self.pos = end;
        self.push(start, end, kind);
    }

    fn dollar(&mut self) {
        let start = self.pos;
        let double = self.peek(1) == Some(b'$');
        let len = if double { 2 } else { 1 };
        match (self.math, double) {
            (MathMode::None, true) => self.math = MathMode::Display,
            (MathMode::None, false) => self.math = MathMode::Inline,
            (MathMode::Display, _) => self.math = MathMode::None,
            (MathMode::Inline, _) => {
                self.math = MathMode::None;
                // `$$` while in inline math closes inline and is otherwise odd; treat as one.
                self.pos += 1;
                self.push(start, self.pos, TokenKind::MathDelimiter);
                return;
            }
            // `$` inside \[..\] or an environment is an error in TeX; keep the mode.
            _ => {}
        }
        self.pos += len;
        self.push(start, self.pos, TokenKind::MathDelimiter);
    }

    fn command(&mut self) {
        let start = self.pos;
        self.pos += 1; // backslash
        let Some(next) = self.peek(0) else {
            let kind = self.text_kind();
            self.push(start, self.pos, kind);
            return;
        };
        if !next.is_ascii_alphabetic() && next != b'@' {
            // Control symbol: \\, \%, \(, \[, \{, ...
            let clen = self.char_len_at(self.pos);
            self.pos += clen;
            match next {
                b'(' | b'[' if self.math == MathMode::None => {
                    self.math = if next == b'(' {
                        MathMode::Paren
                    } else {
                        MathMode::Bracket
                    };
                    self.push(start, self.pos, TokenKind::MathDelimiter);
                }
                b')' if self.math == MathMode::Paren => {
                    self.math = MathMode::None;
                    self.push(start, self.pos, TokenKind::MathDelimiter);
                }
                b']' if self.math == MathMode::Bracket => {
                    self.math = MathMode::None;
                    self.push(start, self.pos, TokenKind::MathDelimiter);
                }
                _ => {
                    let kind = if self.math == MathMode::None {
                        TokenKind::Command
                    } else {
                        TokenKind::MathCommand
                    };
                    self.push(start, self.pos, kind);
                }
            }
            return;
        }
        let name_start = self.pos;
        while let Some(c) = self.peek(0) {
            if c.is_ascii_alphabetic() || c == b'@' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let name = self.src.get(name_start..self.pos).unwrap_or("");
        // A trailing `*` belongs to the command (\section*).
        if self.peek(0) == Some(b'*') {
            self.pos += 1;
        }

        match name {
            "begin" | "end" => self.environment(start, name == "begin"),
            "verb" | "lstinline" | "mintinline" => {
                self.push(start, self.pos, TokenKind::Command);
                self.inline_verbatim(name == "mintinline");
            }
            _ if REFERENCE_COMMANDS.contains(&name) && self.math == MathMode::None => {
                self.push(start, self.pos, TokenKind::Command);
                self.reference_argument();
            }
            _ => {
                let kind = if self.math != MathMode::None {
                    TokenKind::MathCommand
                } else if SECTIONING.contains(&name) {
                    TokenKind::Sectioning
                } else {
                    TokenKind::Command
                };
                self.push(start, self.pos, kind);
            }
        }
    }

    /// After `\begin` / `\end`: colour `{name}`, switch math/verbatim modes.
    fn environment(&mut self, cmd_start: usize, is_begin: bool) {
        self.push(cmd_start, self.pos, TokenKind::EnvironmentCommand);
        // Optional whitespace, then `{name}`.
        let mut p = self.pos;
        while matches!(self.bytes.get(p), Some(b' ' | b'\t')) {
            p += 1;
        }
        if self.bytes.get(p) != Some(&b'{') {
            return;
        }
        let Some(close_rel) = self
            .bytes
            .get(p + 1..)
            .and_then(|r| r.iter().position(|&c| c == b'}' || c == b'\n'))
        else {
            return;
        };
        let close = p + 1 + close_rel;
        if self.bytes.get(close) != Some(&b'}') {
            return;
        }
        let name = self.src.get(p + 1..close).unwrap_or("").trim();
        let ws_kind = self.text_kind();
        self.push(self.pos, p, ws_kind);
        self.push(p, p + 1, TokenKind::Brace);
        self.push(p + 1, close, TokenKind::EnvironmentName);
        self.push(close, close + 1, TokenKind::Brace);
        self.pos = close + 1;

        if MATH_ENVIRONMENTS.contains(&name) {
            if is_begin {
                if self.math == MathMode::None {
                    self.math = MathMode::Env;
                }
                if self.math == MathMode::Env {
                    self.math_env_depth = self.math_env_depth.saturating_add(1);
                }
            } else if self.math == MathMode::Env {
                self.math_env_depth = self.math_env_depth.saturating_sub(1);
                if self.math_env_depth == 0 {
                    self.math = MathMode::None;
                }
            }
        } else if is_begin && VERBATIM_ENVIRONMENTS.contains(&name) {
            self.verbatim_environment(name);
        }
    }

    /// Body of a verbatim-like environment up to its `\end{name}`.
    fn verbatim_environment(&mut self, name: &str) {
        let end_tag = format!("\\end{{{name}}}");
        let start = self.pos;
        let end = self
            .src
            .get(start..)
            .and_then(|rest| rest.find(&end_tag))
            .map_or(self.bytes.len(), |i| start + i);
        self.pos = end;
        self.push(start, end, TokenKind::Verbatim);
    }

    /// `\verb|...|` style: the next char is the delimiter (or `{` for minted/lstinline).
    fn inline_verbatim(&mut self, skip_language_arg: bool) {
        if skip_language_arg && self.peek(0) == Some(b'{') {
            // \mintinline{lang}{code}
            if let Some(close) = self
                .bytes
                .get(self.pos..)
                .and_then(|r| r.iter().position(|&c| c == b'}'))
            {
                let s = self.pos;
                self.push(s, s + 1, TokenKind::Brace);
                self.push(s + 1, s + close, TokenKind::Text);
                self.push(s + close, s + close + 1, TokenKind::Brace);
                self.pos = s + close + 1;
            }
        }
        if self.peek(0) == Some(b'*') {
            self.pos += 1;
        }
        let Some(delim) = self.peek(0) else { return };
        if delim.is_ascii_whitespace() || !delim.is_ascii() {
            return;
        }
        let close_delim = if delim == b'{' { b'}' } else { delim };
        let start = self.pos;
        let line_end = self.line_end(start);
        let close = self
            .bytes
            .get(start + 1..line_end)
            .and_then(|r| r.iter().position(|&c| c == close_delim))
            .map_or(line_end, |i| start + 1 + i + 1);
        self.pos = close.min(self.bytes.len());
        self.push(start, self.pos, TokenKind::Verbatim);
    }

    /// `{key, key2}` after a reference command, with an optional `[..]` before it.
    fn reference_argument(&mut self) {
        let mut p = self.pos;
        // Up to two optional arguments: \cite[p.~3][see]{key}
        for _ in 0..2 {
            if self.bytes.get(p) == Some(&b'[') {
                let Some(close) = self
                    .bytes
                    .get(p..)
                    .and_then(|r| r.iter().position(|&c| c == b']' || c == b'\n'))
                else {
                    return;
                };
                if self.bytes.get(p + close) != Some(&b']') {
                    return;
                }
                self.push(p, p + 1, TokenKind::Bracket);
                self.push(p + 1, p + close, TokenKind::Text);
                self.push(p + close, p + close + 1, TokenKind::Bracket);
                p += close + 1;
            }
        }
        self.pos = p;
        if self.bytes.get(p) != Some(&b'{') {
            return;
        }
        let Some(close) = self
            .bytes
            .get(p..)
            .and_then(|r| r.iter().position(|&c| c == b'}' || c == b'\n'))
        else {
            return;
        };
        if self.bytes.get(p + close) != Some(&b'}') {
            return;
        }
        self.push(p, p + 1, TokenKind::Brace);
        self.push(p + 1, p + close, TokenKind::Reference);
        self.push(p + close, p + close + 1, TokenKind::Brace);
        self.pos = p + close + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(String, TokenKind)> {
        highlight(text)
            .into_iter()
            .map(|s| (text[s.range.clone()].to_string(), s.kind))
            .collect()
    }

    fn covers(text: &str) {
        let spans = highlight(text);
        let mut at = 0;
        for s in &spans {
            assert_eq!(s.range.start, at, "gap in {text:?}");
            assert!(s.range.end > s.range.start);
            assert!(text.is_char_boundary(s.range.start) && text.is_char_boundary(s.range.end));
            at = s.range.end;
        }
        assert_eq!(at, text.len());
    }

    #[test]
    fn commands_comments_and_text() {
        let k = kinds("Hello \\emph{world} % note\n");
        assert_eq!(k[0], ("Hello ".into(), TokenKind::Text));
        assert_eq!(k[1], ("\\emph".into(), TokenKind::Command));
        assert_eq!(k[2], ("{".into(), TokenKind::Brace));
        assert!(k.contains(&("% note".into(), TokenKind::Comment)));
    }

    #[test]
    fn escaped_percent_is_not_a_comment() {
        let k = kinds("50\\% off");
        assert!(k.iter().all(|(_, kind)| *kind != TokenKind::Comment));
    }

    #[test]
    fn magic_comment() {
        let k = kinds("% !TEX program = xelatex\n");
        assert_eq!(k[0].1, TokenKind::MagicComment);
    }

    #[test]
    fn inline_and_display_math() {
        let k = kinds("a $x^2 + \\alpha$ b \\[ y \\] c");
        assert!(k.contains(&("$".into(), TokenKind::MathDelimiter)));
        assert!(k.contains(&("\\alpha".into(), TokenKind::MathCommand)));
        assert!(k.contains(&("\\[".into(), TokenKind::MathDelimiter)));
        assert_eq!(k.last().map(|x| x.1), Some(TokenKind::Text));
    }

    #[test]
    fn math_environment() {
        let k = kinds("\\begin{equation}\n e = mc^2\n\\end{equation} after");
        assert!(k.contains(&("equation".into(), TokenKind::EnvironmentName)));
        assert!(
            k.iter()
                .any(|(t, kind)| t.contains("mc") && *kind == TokenKind::Math)
        );
        assert_eq!(k.last().map(|x| x.1), Some(TokenKind::Text));
    }

    #[test]
    fn references_and_sectioning() {
        let k = kinds("\\section{Intro}\\label{sec:intro} see \\cite[p.~2]{knuth84,lamport}");
        assert_eq!(k[0], ("\\section".into(), TokenKind::Sectioning));
        assert!(k.contains(&("sec:intro".into(), TokenKind::Reference)));
        assert!(k.contains(&("knuth84,lamport".into(), TokenKind::Reference)));
    }

    #[test]
    fn verbatim() {
        let k = kinds("\\begin{verbatim}\n$ % not math\n\\end{verbatim}\n\\verb|$x%| y");
        assert!(
            k.iter()
                .any(|(t, kind)| t.contains("not math") && *kind == TokenKind::Verbatim)
        );
        assert!(k.contains(&("|$x%|".into(), TokenKind::Verbatim)));
        assert_eq!(k.last().map(|x| x.1), Some(TokenKind::Text));
    }

    #[test]
    fn always_covers_input() {
        for t in [
            "",
            "\\",
            "$",
            "$$",
            "\\begin{",
            "\\begin{equation",
            "\\verb",
            "\\verb|abc",
            "\\cite[",
            "\\label{x",
            "中文 \\section{标题} $数学$ % 注释",
            "\\é",
            "{}[]&~^_#",
            "\\begin{verbatim}never closed",
        ] {
            covers(t);
        }
    }

    #[test]
    fn garbage_never_panics() {
        let mut seed = 0x1234_5678_u64;
        let alphabet = [
            '\\', '$', '{', '}', '[', ']', '%', 'a', 'b', '\n', ' ', '中', '(', ')',
        ];
        for _ in 0..500 {
            let mut s = String::new();
            for _ in 0..64 {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.push(alphabet[(seed >> 33) as usize % alphabet.len()]);
            }
            covers(&s);
        }
    }
}
