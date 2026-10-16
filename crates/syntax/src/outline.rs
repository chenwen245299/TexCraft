//! Document structure: sections, labels, floats, included files and bibliographies.

/// What an [`OutlineItem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutlineKind {
    /// `\part`
    Part,
    /// `\chapter`
    Chapter,
    /// `\section`
    Section,
    /// `\subsection`
    Subsection,
    /// `\subsubsection`
    Subsubsection,
    /// `\paragraph`
    Paragraph,
    /// `\subparagraph`
    Subparagraph,
    /// A beamer `frame` (titled by `\frametitle` or the `{title}` argument).
    Frame,
    /// A `figure` / `figure*` environment, titled by its `\caption`.
    Figure,
    /// A `table` / `table*` environment, titled by its `\caption`.
    Table,
    /// `\label{key}`
    Label,
    /// `\input{file}`, `\include{file}`, `\subfile{file}`, `\import{dir}{file}`.
    Input,
    /// `\bibliography{a,b}` / `\addbibresource{x.bib}`.
    Bibliography,
    /// A `% TODO` / `\todo{...}` marker.
    Todo,
}

impl OutlineKind {
    /// Nesting depth for sectioning kinds (part = 0); other kinds sit below the current section.
    pub fn level(self) -> Option<u8> {
        Some(match self {
            Self::Part => 0,
            Self::Chapter => 1,
            Self::Section => 2,
            Self::Subsection => 3,
            Self::Subsubsection => 4,
            Self::Paragraph => 5,
            Self::Subparagraph => 6,
            Self::Frame => 3,
            _ => return None,
        })
    }

    /// Whether this is a sectioning item (it opens a level in the outline tree).
    pub fn is_sectioning(self) -> bool {
        self.level().is_some()
    }
}

/// One entry of the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    /// What it is.
    pub kind: OutlineKind,
    /// Display title (simple formatting commands removed), or the label key / file name.
    pub title: String,
    /// 1-based source line.
    pub line: u32,
    /// Byte offset of the command in the source.
    pub offset: usize,
    /// `\section*` and friends.
    pub starred: bool,
}

const SECTION_COMMANDS: &[(&str, OutlineKind)] = &[
    ("part", OutlineKind::Part),
    ("chapter", OutlineKind::Chapter),
    ("section", OutlineKind::Section),
    ("subsection", OutlineKind::Subsection),
    ("subsubsection", OutlineKind::Subsubsection),
    ("paragraph", OutlineKind::Paragraph),
    ("subparagraph", OutlineKind::Subparagraph),
];

/// Extracts the outline of one source file, in source order.
pub fn outline(text: &str) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    let mut line_starts = LineIndex::new(text);
    // Open float environment waiting for its caption: (index into items).
    let mut open_float: Option<usize> = None;
    let mut open_frame: Option<usize> = None;

    for (offset, name) in commands(text) {
        let line = line_starts.line_of(offset);
        let after = offset + 1 + name.len();
        let starred = text.as_bytes().get(after) == Some(&b'*');
        let args_at = if starred { after + 1 } else { after };

        if let Some((_, kind)) = SECTION_COMMANDS.iter().find(|(n, _)| *n == name) {
            let title = skip_optional(text, args_at)
                .and_then(|p| braced_argument(text, p))
                .map(|(t, _)| clean_title(t))
                .unwrap_or_default();
            items.push(OutlineItem {
                kind: *kind,
                title,
                line,
                offset,
                starred,
            });
            continue;
        }

        match name {
            "begin" => {
                let Some((env, end)) = braced_argument(text, args_at) else {
                    continue;
                };
                let env = env.trim();
                let kind = match env {
                    "figure" | "figure*" | "wrapfigure" | "subfigure" => Some(OutlineKind::Figure),
                    "table" | "table*" | "wraptable" => Some(OutlineKind::Table),
                    "frame" => Some(OutlineKind::Frame),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let mut title = String::new();
                    if kind == OutlineKind::Frame {
                        // \begin{frame}[opts]{Title}
                        if let Some((t, _)) = skip_optional(text, end)
                            .and_then(|p| braced_argument_same_line(text, p))
                        {
                            title = clean_title(t);
                        }
                    }
                    items.push(OutlineItem {
                        kind,
                        title,
                        line,
                        offset,
                        starred: false,
                    });
                    if kind == OutlineKind::Frame {
                        open_frame = Some(items.len() - 1);
                    } else {
                        open_float = Some(items.len() - 1);
                    }
                }
            }
            "end" => {
                if let Some((env, _)) = braced_argument(text, args_at) {
                    match env.trim() {
                        "frame" => open_frame = None,
                        "figure" | "figure*" | "table" | "table*" | "wrapfigure" | "wraptable"
                        | "subfigure" => open_float = None,
                        _ => {}
                    }
                }
            }
            "caption" => {
                if let Some(idx) = open_float
                    && let Some(item) = items.get_mut(idx)
                    && item.title.is_empty()
                {
                    item.title = skip_optional(text, args_at)
                        .and_then(|p| braced_argument(text, p))
                        .map(|(t, _)| clean_title(t))
                        .unwrap_or_default();
                }
            }
            "frametitle" => {
                if let Some(idx) = open_frame
                    && let Some(item) = items.get_mut(idx)
                    && item.title.is_empty()
                {
                    item.title = braced_argument(text, args_at)
                        .map(|(t, _)| clean_title(t))
                        .unwrap_or_default();
                }
            }
            "label" => {
                if let Some((key, _)) = braced_argument(text, args_at) {
                    items.push(OutlineItem {
                        kind: OutlineKind::Label,
                        title: key.trim().to_string(),
                        line,
                        offset,
                        starred: false,
                    });
                }
            }
            "input" | "include" | "subfile" | "includeonly" | "import" | "subimport" => {
                if name == "includeonly" {
                    continue;
                }
                let first = braced_argument(text, args_at);
                let title = if matches!(name, "import" | "subimport") {
                    first.and_then(|(dir, end)| {
                        braced_argument(text, end)
                            .map(|(f, _)| format!("{}{}", dir.trim(), f.trim()))
                    })
                } else {
                    first.map(|(f, _)| f.trim().to_string())
                };
                if let Some(title) = title {
                    items.push(OutlineItem {
                        kind: OutlineKind::Input,
                        title,
                        line,
                        offset,
                        starred: false,
                    });
                }
            }
            "bibliography" | "addbibresource" => {
                if let Some((files, _)) =
                    skip_optional(text, args_at).and_then(|p| braced_argument(text, p))
                {
                    items.push(OutlineItem {
                        kind: OutlineKind::Bibliography,
                        title: files.trim().to_string(),
                        line,
                        offset,
                        starred: false,
                    });
                }
            }
            "todo" => {
                let title = skip_optional(text, args_at)
                    .and_then(|p| braced_argument(text, p))
                    .map(|(t, _)| clean_title(t))
                    .unwrap_or_default();
                items.push(OutlineItem {
                    kind: OutlineKind::Todo,
                    title,
                    line,
                    offset,
                    starred: false,
                });
            }
            _ => {}
        }
    }

    // `% TODO ...` comments.
    for (idx, l) in text.lines().enumerate() {
        if let Some(pos) = l.find('%') {
            let comment = l.get(pos + 1..).unwrap_or("").trim_start();
            let marker = ["TODO", "FIXME"].into_iter().find(|m| {
                comment
                    .get(..m.len())
                    .is_some_and(|p| p.eq_ignore_ascii_case(m))
            });
            if let Some(marker) = marker {
                let title = comment
                    .get(marker.len()..)
                    .unwrap_or("")
                    .trim_start_matches([':', ' '])
                    .to_string();
                let line = u32::try_from(idx + 1).unwrap_or(u32::MAX);
                items.push(OutlineItem {
                    kind: OutlineKind::Todo,
                    title,
                    line,
                    offset: line_starts.offset_of_line(line),
                    starred: false,
                });
            }
        }
    }
    items.sort_by_key(|i| i.offset);
    items
}

/// Every `\label{key}` in `text` with its 1-based line.
pub fn labels(text: &str) -> Vec<(String, u32)> {
    outline(text)
        .into_iter()
        .filter(|i| i.kind == OutlineKind::Label)
        .map(|i| (i.title, i.line))
        .collect()
}

/// User-defined commands (`\newcommand{\foo}`, `\renewcommand\foo`, `\def\foo`,
/// `\DeclareMathOperator{\foo}`), without the backslash.
pub fn defined_commands(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (offset, name) in commands(text) {
        if !matches!(
            name,
            "newcommand"
                | "renewcommand"
                | "providecommand"
                | "def"
                | "DeclareMathOperator"
                | "NewDocumentCommand"
        ) {
            continue;
        }
        let mut p = offset + 1 + name.len();
        if text.as_bytes().get(p) == Some(&b'*') {
            p += 1;
        }
        let rest = text.get(p..).unwrap_or("");
        let rest = rest.trim_start_matches('{').trim_start();
        if let Some(stripped) = rest.strip_prefix('\\') {
            let n: String = stripped
                .chars()
                .take_while(|c| c.is_ascii_alphabetic() || *c == '@')
                .collect();
            if !n.is_empty() && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// All `(byte offset of the backslash, command name)` pairs, skipping comments and `\\`.
pub fn commands(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    std::iter::from_fn(move || {
        while let Some(&b) = bytes.get(i) {
            match b {
                b'%' => {
                    // Skip to end of line.
                    i = bytes
                        .get(i..)
                        .and_then(|r| r.iter().position(|&c| c == b'\n'))
                        .map_or(bytes.len(), |p| i + p);
                }
                b'\\' => {
                    let start = i;
                    let name_start = i + 1;
                    let mut j = name_start;
                    while bytes
                        .get(j)
                        .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'@')
                    {
                        j += 1;
                    }
                    if j == name_start {
                        // Control symbol (\\, \%, ...): skip it and its character.
                        i = name_start
                            + text
                                .get(name_start..)
                                .and_then(|s| s.chars().next())
                                .map_or(1, char::len_utf8);
                        continue;
                    }
                    i = j;
                    if let Some(name) = text.get(name_start..j) {
                        return Some((start, name));
                    }
                }
                _ => i += 1,
            }
        }
        None
    })
}

/// If an optional `[...]` argument starts at `pos` (after spaces), returns the position after it;
/// otherwise `pos` itself.
pub fn skip_optional(text: &str, pos: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut p = pos;
    while matches!(bytes.get(p), Some(b' ' | b'\t')) {
        p += 1;
    }
    if bytes.get(p) != Some(&b'[') {
        return Some(pos);
    }
    let mut depth = 0i32;
    while let Some(&b) = bytes.get(p) {
        match b {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(p + 1);
                }
            }
            b'\\' => p += 1,
            _ => {}
        }
        p += 1;
    }
    None
}

/// The balanced `{...}` argument starting at `pos` (after spaces/newline): its content and the
/// position just after the closing brace.
pub fn braced_argument(text: &str, pos: usize) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    let mut p = pos;
    while matches!(bytes.get(p), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        p += 1;
    }
    if bytes.get(p) != Some(&b'{') {
        return None;
    }
    let open = p;
    let mut depth = 0i32;
    while let Some(&b) = bytes.get(p) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return text.get(open + 1..p).map(|s| (s, p + 1));
                }
            }
            b'\\' => p += 1,
            _ => {}
        }
        p += 1;
        // Arguments longer than this are almost certainly unbalanced braces.
        if p - open > 4096 {
            return None;
        }
    }
    None
}

fn braced_argument_same_line(text: &str, pos: usize) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    let mut p = pos;
    while matches!(bytes.get(p), Some(b' ' | b'\t')) {
        p += 1;
    }
    braced_argument(text, p)
}

/// Turns a title argument into display text: drops `\label{..}`, unwraps simple formatting
/// commands, collapses whitespace.
pub fn clean_title(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                let rest = raw.get(i + 1..).unwrap_or("");
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .collect();
                if name.is_empty() {
                    // Control symbol: keep the symbol itself (\& -> &, \\ -> space).
                    if let Some((_, sym)) = chars.next() {
                        out.push(if sym == '\\' { ' ' } else { sym });
                    }
                    continue;
                }
                for _ in 0..name.chars().count() {
                    chars.next();
                }
                if matches!(
                    name.as_str(),
                    "label" | "index" | "footnote" | "protect" | "hspace" | "vspace"
                ) {
                    if matches!(name.as_str(), "protect") {
                        continue;
                    }
                    // Skip its argument.
                    if let Some(&(j, '{')) = chars.peek()
                        && let Some((_, end)) = braced_argument(raw, j)
                    {
                        while chars.peek().is_some_and(|&(k, _)| k < end) {
                            chars.next();
                        }
                    }
                    continue;
                }
                match name.as_str() {
                    "LaTeX" => out.push_str("LaTeX"),
                    "TeX" => out.push_str("TeX"),
                    "ldots" | "dots" => out.push('…'),
                    _ => {}
                }
            }
            '{' | '}' => {}
            '~' => out.push(' '),
            '\n' | '\r' | '\t' => out.push(' '),
            _ => out.push(c),
        }
    }
    let mut collapsed = String::with_capacity(out.len());
    let mut last_space = true;
    for c in out.chars() {
        if c == ' ' {
            if !last_space {
                collapsed.push(' ');
            }
            last_space = true;
        } else {
            collapsed.push(c);
            last_space = false;
        }
    }
    collapsed.trim_end().to_string()
}

/// Maps byte offsets to 1-based line numbers.
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    /// Indexes `text`.
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(i, _)| i + 1),
        );
        Self { starts }
    }

    /// 1-based line containing byte `offset`.
    pub fn line_of(&mut self, offset: usize) -> u32 {
        let idx = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        u32::try_from(idx + 1).unwrap_or(u32::MAX)
    }

    /// Byte offset of the start of 1-based `line` (clamped).
    pub fn offset_of_line(&self, line: u32) -> usize {
        let idx = (line.max(1) as usize).saturating_sub(1);
        self.starts
            .get(idx)
            .or(self.starts.last())
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r"\documentclass{article}
% !TEX program = pdflatex
\begin{document}
\section{Introduction}\label{sec:intro}
Text. % TODO: write more
\subsection*{The \emph{first} part}
\begin{figure}[h]
  \includegraphics{a}
  \caption[short]{A \textbf{nice} plot}\label{fig:plot}
\end{figure}
\input{chapters/two}
\begin{table}\caption{Numbers}\end{table}
\bibliography{refs,more}
\end{document}
";

    #[test]
    fn extracts_structure() {
        let o = outline(DOC);
        let summary: Vec<(OutlineKind, &str, u32)> = o
            .iter()
            .map(|i| (i.kind, i.title.as_str(), i.line))
            .collect();
        assert_eq!(
            summary,
            vec![
                (OutlineKind::Section, "Introduction", 4),
                (OutlineKind::Label, "sec:intro", 4),
                (OutlineKind::Todo, "write more", 5),
                (OutlineKind::Subsection, "The first part", 6),
                (OutlineKind::Figure, "A nice plot", 7),
                (OutlineKind::Label, "fig:plot", 9),
                (OutlineKind::Input, "chapters/two", 11),
                (OutlineKind::Table, "Numbers", 12),
                (OutlineKind::Bibliography, "refs,more", 13),
            ]
        );
        assert!(
            o.iter()
                .find(|i| i.kind == OutlineKind::Subsection)
                .is_some_and(|i| i.starred)
        );
    }

    #[test]
    fn commented_out_sections_are_ignored() {
        let o = outline("% \\section{Old}\n\\section{New}");
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].title, "New");
    }

    #[test]
    fn beamer_frames() {
        let o = outline(
            "\\begin{frame}{Overview}\n\\end{frame}\n\\begin{frame}\n\\frametitle{Results}\n\\end{frame}",
        );
        let titles: Vec<&str> = o.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["Overview", "Results"]);
    }

    #[test]
    fn user_commands() {
        let c = defined_commands(
            "\\newcommand{\\R}{\\mathbb{R}}\n\\newcommand\\vect[1]{#1}\\def\\foo{x}\\DeclareMathOperator{\\Tr}{Tr}",
        );
        assert_eq!(c, vec!["R", "vect", "foo", "Tr"]);
    }

    #[test]
    fn unbalanced_input_never_panics() {
        for t in [
            "\\section{",
            "\\section",
            "\\begin{figure}\\caption{",
            "\\label",
            "\\import{a}",
            "{{{{",
            "\\section{\\}",
        ] {
            let _ = outline(t);
            let _ = defined_commands(t);
        }
    }

    #[test]
    fn clean_titles() {
        assert_eq!(
            clean_title("The \\LaTeX{} \\emph{way}\\label{x}"),
            "The LaTeX way"
        );
        assert_eq!(clean_title("A \\& B~C"), "A & B C");
        assert_eq!(clean_title("中文 标题"), "中文 标题");
    }
}
