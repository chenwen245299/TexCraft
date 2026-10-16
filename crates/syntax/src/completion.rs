//! Autocompletion: what the user is typing at the cursor, and the built-in command and
//! environment catalogue.
//!
//! Snippets use `$1`, `$2`, ... for placeholders and `$0` for the final cursor position;
//! [`expand_snippet`] turns one into plain text plus the cursor offset.

/// What kind of thing is being completed at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
    /// After `\` : a command name. `start` is the byte offset of the backslash.
    Command {
        /// Typed name so far (without the backslash).
        prefix: String,
        /// Byte offset of the backslash.
        start: usize,
    },
    /// Inside `\begin{` / `\end{`.
    Environment {
        /// Typed name so far.
        prefix: String,
        /// Byte offset where the name starts.
        start: usize,
    },
    /// Inside `\ref{`, `\eqref{`, `\autoref{`, `\cref{`, `\pageref{`, ...
    Reference {
        /// Typed key so far (after the last comma).
        prefix: String,
        /// Byte offset where that key starts.
        start: usize,
    },
    /// Inside `\cite{`, `\citep{`, `\parencite{`, ... (after the last comma).
    Citation {
        /// Typed key so far.
        prefix: String,
        /// Byte offset where that key starts.
        start: usize,
    },
    /// Inside `\input{`, `\include{`, `\includegraphics{`, `\bibliography{`, ...
    File {
        /// Typed path so far.
        prefix: String,
        /// Byte offset where the path starts.
        start: usize,
        /// The command (`input`, `includegraphics`, ...), to filter by file type.
        command: String,
    },
    /// Nothing to complete.
    None,
}

const REF_COMMANDS: &[&str] = &[
    "ref",
    "eqref",
    "pageref",
    "autoref",
    "nameref",
    "cref",
    "Cref",
    "vref",
    "labelcref",
];
const CITE_COMMANDS: &[&str] = &[
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
    "fullcite",
    "smartcite",
];
const FILE_COMMANDS: &[&str] = &[
    "input",
    "include",
    "includegraphics",
    "subfile",
    "bibliography",
    "addbibresource",
    "includepdf",
    "lstinputlisting",
];

/// Works out what is being completed when the cursor is at byte offset `cursor`.
pub fn context_at(text: &str, cursor: usize) -> CompletionContext {
    let cursor = cursor.min(text.len());
    if !text.is_char_boundary(cursor) {
        return CompletionContext::None;
    }
    let line_start = text
        .get(..cursor)
        .and_then(|s| s.rfind('\n'))
        .map_or(0, |i| i + 1);
    let Some(line) = text.get(line_start..cursor) else {
        return CompletionContext::None;
    };
    // Inside a comment: nothing.
    if in_comment(line) {
        return CompletionContext::None;
    }

    // `\name` directly before the cursor.
    let name_len = line
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphabetic() || *b == b'@')
        .count();
    let before_name = line.len() - name_len;
    if line.as_bytes().get(before_name.wrapping_sub(1)) == Some(&b'\\') && before_name > 0 {
        let backslashes = line
            .get(..before_name)
            .unwrap_or("")
            .bytes()
            .rev()
            .take_while(|b| *b == b'\\')
            .count();
        if backslashes % 2 == 1 {
            return CompletionContext::Command {
                prefix: line.get(before_name..).unwrap_or("").to_string(),
                start: line_start + before_name - 1,
            };
        }
    }

    // Inside `\cmd[...]{arg` : find the last unclosed `{` on this line.
    let mut depth = 0i32;
    let bytes = line.as_bytes();
    let mut open = None;
    for i in (0..bytes.len()).rev() {
        match bytes.get(i) {
            Some(b'}') => depth += 1,
            Some(b'{') => {
                if depth == 0 {
                    open = Some(i);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    let Some(open) = open else {
        return CompletionContext::None;
    };
    // Command before the brace, skipping optional arguments and a star.
    let mut p = open;
    loop {
        if p > 0 && bytes.get(p - 1) == Some(&b']') {
            let Some(o) = line.get(..p - 1).and_then(|s| s.rfind('[')) else {
                return CompletionContext::None;
            };
            p = o;
        } else {
            break;
        }
    }
    if p > 0 && bytes.get(p - 1) == Some(&b'*') {
        p -= 1;
    }
    let cmd_len = line
        .get(..p)
        .unwrap_or("")
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphabetic())
        .count();
    if cmd_len == 0 || p < cmd_len + 1 || bytes.get(p - cmd_len - 1) != Some(&b'\\') {
        return CompletionContext::None;
    }
    let cmd = line.get(p - cmd_len..p).unwrap_or("");
    let arg = line.get(open + 1..).unwrap_or("");
    let key_start_in_arg = arg.rfind(',').map_or(0, |i| i + 1);
    let key = arg.get(key_start_in_arg..).unwrap_or("").trim_start();
    let key_start = line_start
        + open
        + 1
        + (arg.len() - arg.get(key_start_in_arg..).unwrap_or("").len())
        + (arg.get(key_start_in_arg..).unwrap_or("").len() - key.len());

    if matches!(cmd, "begin" | "end") {
        return CompletionContext::Environment {
            prefix: arg.to_string(),
            start: line_start + open + 1,
        };
    }
    if REF_COMMANDS.contains(&cmd) {
        return CompletionContext::Reference {
            prefix: key.to_string(),
            start: key_start,
        };
    }
    if CITE_COMMANDS.contains(&cmd) {
        return CompletionContext::Citation {
            prefix: key.to_string(),
            start: key_start,
        };
    }
    if FILE_COMMANDS.contains(&cmd) {
        let (prefix, start) = if matches!(cmd, "bibliography") {
            (key.to_string(), key_start)
        } else {
            (arg.to_string(), line_start + open + 1)
        };
        return CompletionContext::File {
            prefix,
            start,
            command: cmd.to_string(),
        };
    }
    CompletionContext::None
}

fn in_comment(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'\\' => i += 2,
            b'%' => return true,
            _ => i += 1,
        }
    }
    false
}

/// A built-in command for completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandInfo {
    /// Name without the backslash.
    pub name: &'static str,
    /// Text inserted after the backslash, with `$n` placeholders.
    pub snippet: &'static str,
    /// Short description or package.
    pub detail: &'static str,
}

/// A built-in environment for completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvironmentInfo {
    /// Environment name.
    pub name: &'static str,
    /// Text inserted after `\begin{name}` (arguments), with `$n` placeholders.
    pub arguments: &'static str,
    /// Body inserted between begin and end, with `$n` placeholders (`$0` if empty).
    pub body: &'static str,
    /// Short description or package.
    pub detail: &'static str,
}

macro_rules! cmds {
    ($( $name:literal => $snippet:literal, $detail:literal; )*) => {
        &[ $( CommandInfo { name: $name, snippet: $snippet, detail: $detail }, )* ]
    };
}

/// The built-in command catalogue.
pub const COMMANDS: &[CommandInfo] = cmds! {
    // Document structure
    "documentclass" => "documentclass[$1]{$2}", "preamble";
    "usepackage" => "usepackage{$1}", "preamble";
    "title" => "title{$1}", "title page";
    "author" => "author{$1}", "title page";
    "date" => "date{$1}", "title page";
    "maketitle" => "maketitle", "title page";
    "tableofcontents" => "tableofcontents", "contents";
    "listoffigures" => "listoffigures", "contents";
    "listoftables" => "listoftables", "contents";
    "part" => "part{$1}", "sectioning";
    "chapter" => "chapter{$1}", "sectioning";
    "section" => "section{$1}", "sectioning";
    "subsection" => "subsection{$1}", "sectioning";
    "subsubsection" => "subsubsection{$1}", "sectioning";
    "paragraph" => "paragraph{$1}", "sectioning";
    "subparagraph" => "subparagraph{$1}", "sectioning";
    "appendix" => "appendix", "sectioning";
    "abstract" => "abstract", "abstract";
    "begin" => "begin{$1}", "environment";
    "end" => "end{$1}", "environment";
    "input" => "input{$1}", "include a file";
    "include" => "include{$1}", "include a file (new page)";
    "includeonly" => "includeonly{$1}", "preamble";
    "newcommand" => "newcommand{\\$1}{$2}", "define a command";
    "renewcommand" => "renewcommand{\\$1}{$2}", "redefine a command";
    "newenvironment" => "newenvironment{$1}{$2}{$3}", "define an environment";
    "DeclareMathOperator" => "DeclareMathOperator{\\$1}{$2}", "amsmath";
    "setlength" => "setlength{$1}{$2}", "lengths";
    "newpage" => "newpage", "page break";
    "clearpage" => "clearpage", "page break";
    "pagebreak" => "pagebreak", "page break";
    "linebreak" => "linebreak", "line break";
    "newline" => "newline", "line break";
    "noindent" => "noindent", "paragraph";
    "indent" => "indent", "paragraph";
    "par" => "par", "paragraph";
    "vspace" => "vspace{$1}", "spacing";
    "hspace" => "hspace{$1}", "spacing";
    "vfill" => "vfill", "spacing";
    "hfill" => "hfill", "spacing";
    "smallskip" => "smallskip", "spacing";
    "medskip" => "medskip", "spacing";
    "bigskip" => "bigskip", "spacing";
    "quad" => "quad", "spacing";
    "qquad" => "qquad", "spacing";
    "centering" => "centering", "alignment";
    "raggedright" => "raggedright", "alignment";
    "raggedleft" => "raggedleft", "alignment";
    // Text formatting
    "textbf" => "textbf{$1}", "bold";
    "textit" => "textit{$1}", "italic";
    "emph" => "emph{$1}", "emphasis";
    "underline" => "underline{$1}", "underline";
    "texttt" => "texttt{$1}", "typewriter";
    "textsc" => "textsc{$1}", "small caps";
    "textsf" => "textsf{$1}", "sans serif";
    "textrm" => "textrm{$1}", "roman";
    "textsuperscript" => "textsuperscript{$1}", "superscript";
    "textsubscript" => "textsubscript{$1}", "subscript";
    "textcolor" => "textcolor{$1}{$2}", "xcolor";
    "colorbox" => "colorbox{$1}{$2}", "xcolor";
    "color" => "color{$1}", "xcolor";
    "tiny" => "tiny", "font size";
    "scriptsize" => "scriptsize", "font size";
    "footnotesize" => "footnotesize", "font size";
    "small" => "small", "font size";
    "normalsize" => "normalsize", "font size";
    "large" => "large", "font size";
    "Large" => "Large", "font size";
    "LARGE" => "LARGE", "font size";
    "huge" => "huge", "font size";
    "Huge" => "Huge", "font size";
    "bfseries" => "bfseries", "bold (declaration)";
    "itshape" => "itshape", "italic (declaration)";
    "ttfamily" => "ttfamily", "typewriter (declaration)";
    "footnote" => "footnote{$1}", "footnote";
    "url" => "url{$1}", "hyperref/url";
    "href" => "href{$1}{$2}", "hyperref";
    "verb" => "verb|$1|", "inline verbatim";
    "today" => "today", "date";
    "LaTeX" => "LaTeX", "logo";
    "TeX" => "TeX", "logo";
    "ldots" => "ldots", "ellipsis";
    "item" => "item $0", "list item";
    // Cross references and citations
    "label" => "label{$1}", "label";
    "ref" => "ref{$1}", "reference";
    "eqref" => "eqref{$1}", "equation reference";
    "pageref" => "pageref{$1}", "page reference";
    "autoref" => "autoref{$1}", "hyperref";
    "nameref" => "nameref{$1}", "hyperref";
    "cref" => "cref{$1}", "cleveref";
    "Cref" => "Cref{$1}", "cleveref";
    "cite" => "cite{$1}", "citation";
    "citep" => "citep{$1}", "natbib";
    "citet" => "citet{$1}", "natbib";
    "parencite" => "parencite{$1}", "biblatex";
    "textcite" => "textcite{$1}", "biblatex";
    "autocite" => "autocite{$1}", "biblatex";
    "nocite" => "nocite{$1}", "citation";
    "bibliography" => "bibliography{$1}", "BibTeX";
    "bibliographystyle" => "bibliographystyle{$1}", "BibTeX";
    "addbibresource" => "addbibresource{$1}", "biblatex";
    "printbibliography" => "printbibliography", "biblatex";
    // Floats and graphics
    "includegraphics" => "includegraphics[width=$1\\linewidth]{$2}", "graphicx";
    "caption" => "caption{$1}", "float caption";
    "hline" => "hline", "table rule";
    "cline" => "cline{$1}", "table rule";
    "toprule" => "toprule", "booktabs";
    "midrule" => "midrule", "booktabs";
    "bottomrule" => "bottomrule", "booktabs";
    "multicolumn" => "multicolumn{$1}{$2}{$3}", "table";
    "multirow" => "multirow{$1}{$2}{$3}", "multirow";
    // Math: structures
    "frac" => "frac{$1}{$2}", "fraction";
    "dfrac" => "dfrac{$1}{$2}", "amsmath";
    "tfrac" => "tfrac{$1}{$2}", "amsmath";
    "sqrt" => "sqrt{$1}", "root";
    "binom" => "binom{$1}{$2}", "amsmath";
    "sum" => "sum_{$1}^{$2}", "sum";
    "prod" => "prod_{$1}^{$2}", "product";
    "int" => "int_{$1}^{$2}", "integral";
    "iint" => "iint", "amsmath";
    "oint" => "oint", "contour integral";
    "lim" => "lim_{$1}", "limit";
    "infty" => "infty", "infinity";
    "partial" => "partial", "partial";
    "nabla" => "nabla", "nabla";
    "left" => "left", "delimiter";
    "right" => "right", "delimiter";
    "big" => "big", "delimiter size";
    "Big" => "Big", "delimiter size";
    "mathbb" => "mathbb{$1}", "amssymb";
    "mathcal" => "mathcal{$1}", "calligraphic";
    "mathbf" => "mathbf{$1}", "bold math";
    "mathrm" => "mathrm{$1}", "roman math";
    "mathit" => "mathit{$1}", "italic math";
    "mathsf" => "mathsf{$1}", "sans math";
    "mathfrak" => "mathfrak{$1}", "amssymb";
    "boldsymbol" => "boldsymbol{$1}", "amsmath";
    "text" => "text{$1}", "amsmath";
    "operatorname" => "operatorname{$1}", "amsmath";
    "hat" => "hat{$1}", "accent";
    "bar" => "bar{$1}", "accent";
    "tilde" => "tilde{$1}", "accent";
    "vec" => "vec{$1}", "accent";
    "dot" => "dot{$1}", "accent";
    "ddot" => "ddot{$1}", "accent";
    "overline" => "overline{$1}", "accent";
    "underbrace" => "underbrace{$1}_{$2}", "brace";
    "overbrace" => "overbrace{$1}^{$2}", "brace";
    "widehat" => "widehat{$1}", "accent";
    "cdot" => "cdot", "operator";
    "cdots" => "cdots", "dots";
    "vdots" => "vdots", "dots";
    "ddots" => "ddots", "dots";
    "times" => "times", "operator";
    "div" => "div", "operator";
    "pm" => "pm", "operator";
    "mp" => "mp", "operator";
    "circ" => "circ", "operator";
    "otimes" => "otimes", "operator";
    "oplus" => "oplus", "operator";
    "cup" => "cup", "set";
    "cap" => "cap", "set";
    "setminus" => "setminus", "set";
    "subset" => "subset", "set";
    "subseteq" => "subseteq", "set";
    "supset" => "supset", "set";
    "supseteq" => "supseteq", "set";
    "in" => "in", "set";
    "notin" => "notin", "set";
    "emptyset" => "emptyset", "set";
    "varnothing" => "varnothing", "amssymb";
    "forall" => "forall", "logic";
    "exists" => "exists", "logic";
    "neg" => "neg", "logic";
    "land" => "land", "logic";
    "lor" => "lor", "logic";
    "implies" => "implies", "amsmath";
    "iff" => "iff", "amsmath";
    "leq" => "leq", "relation";
    "geq" => "geq", "relation";
    "neq" => "neq", "relation";
    "approx" => "approx", "relation";
    "equiv" => "equiv", "relation";
    "sim" => "sim", "relation";
    "simeq" => "simeq", "relation";
    "propto" => "propto", "relation";
    "ll" => "ll", "relation";
    "gg" => "gg", "relation";
    "to" => "to", "arrow";
    "rightarrow" => "rightarrow", "arrow";
    "leftarrow" => "leftarrow", "arrow";
    "Rightarrow" => "Rightarrow", "arrow";
    "Leftarrow" => "Leftarrow", "arrow";
    "leftrightarrow" => "leftrightarrow", "arrow";
    "Leftrightarrow" => "Leftrightarrow", "arrow";
    "mapsto" => "mapsto", "arrow";
    "uparrow" => "uparrow", "arrow";
    "downarrow" => "downarrow", "arrow";
    "langle" => "langle", "delimiter";
    "rangle" => "rangle", "delimiter";
    "lfloor" => "lfloor", "delimiter";
    "rfloor" => "rfloor", "delimiter";
    "lceil" => "lceil", "delimiter";
    "rceil" => "rceil", "delimiter";
    "sin" => "sin", "function";
    "cos" => "cos", "function";
    "tan" => "tan", "function";
    "log" => "log", "function";
    "ln" => "ln", "function";
    "exp" => "exp", "function";
    "max" => "max", "function";
    "min" => "min", "function";
    "sup" => "sup", "function";
    "inf" => "inf", "function";
    "det" => "det", "function";
    // Greek
    "alpha" => "alpha", "Greek";
    "beta" => "beta", "Greek";
    "gamma" => "gamma", "Greek";
    "Gamma" => "Gamma", "Greek";
    "delta" => "delta", "Greek";
    "Delta" => "Delta", "Greek";
    "epsilon" => "epsilon", "Greek";
    "varepsilon" => "varepsilon", "Greek";
    "zeta" => "zeta", "Greek";
    "eta" => "eta", "Greek";
    "theta" => "theta", "Greek";
    "vartheta" => "vartheta", "Greek";
    "Theta" => "Theta", "Greek";
    "iota" => "iota", "Greek";
    "kappa" => "kappa", "Greek";
    "lambda" => "lambda", "Greek";
    "Lambda" => "Lambda", "Greek";
    "mu" => "mu", "Greek";
    "nu" => "nu", "Greek";
    "xi" => "xi", "Greek";
    "Xi" => "Xi", "Greek";
    "pi" => "pi", "Greek";
    "Pi" => "Pi", "Greek";
    "rho" => "rho", "Greek";
    "varrho" => "varrho", "Greek";
    "sigma" => "sigma", "Greek";
    "Sigma" => "Sigma", "Greek";
    "tau" => "tau", "Greek";
    "upsilon" => "upsilon", "Greek";
    "phi" => "phi", "Greek";
    "varphi" => "varphi", "Greek";
    "Phi" => "Phi", "Greek";
    "chi" => "chi", "Greek";
    "psi" => "psi", "Greek";
    "Psi" => "Psi", "Greek";
    "omega" => "omega", "Greek";
    "Omega" => "Omega", "Greek";
};

macro_rules! envs {
    ($( $name:literal => $args:literal, $body:literal, $detail:literal; )*) => {
        &[ $( EnvironmentInfo { name: $name, arguments: $args, body: $body, detail: $detail }, )* ]
    };
}

/// The built-in environment catalogue.
pub const ENVIRONMENTS: &[EnvironmentInfo] = envs! {
    "document" => "", "$0", "document body";
    "abstract" => "", "$0", "abstract";
    "itemize" => "", "\\item $0", "bulleted list";
    "enumerate" => "", "\\item $0", "numbered list";
    "description" => "", "\\item[$1] $0", "description list";
    "equation" => "", "$0", "numbered equation";
    "equation*" => "", "$0", "amsmath";
    "align" => "", "$0", "amsmath";
    "align*" => "", "$0", "amsmath";
    "gather" => "", "$0", "amsmath";
    "gather*" => "", "$0", "amsmath";
    "multline" => "", "$0", "amsmath";
    "split" => "", "$0", "amsmath";
    "cases" => "", "$1 & $2 \\\\\n$0", "amsmath";
    "matrix" => "", "$0", "amsmath";
    "pmatrix" => "", "$0", "amsmath";
    "bmatrix" => "", "$0", "amsmath";
    "vmatrix" => "", "$0", "amsmath";
    "array" => "{$1}", "$0", "math array";
    "figure" => "[$1htbp]", "\\centering\n\\includegraphics[width=0.8\\linewidth]{$2}\n\\caption{$3}\n\\label{fig:$4}", "float";
    "figure*" => "[htbp]", "\\centering\n$0", "float";
    "table" => "[$1htbp]", "\\centering\n\\caption{$2}\n\\label{tab:$3}\n\\begin{tabular}{$4}\n$0\n\\end{tabular}", "float";
    "table*" => "[htbp]", "\\centering\n$0", "float";
    "tabular" => "{$1}", "$0", "table";
    "tabularx" => "{\\linewidth}{$1}", "$0", "tabularx";
    "center" => "", "$0", "alignment";
    "flushleft" => "", "$0", "alignment";
    "flushright" => "", "$0", "alignment";
    "quote" => "", "$0", "quotation";
    "quotation" => "", "$0", "quotation";
    "verbatim" => "", "$0", "verbatim";
    "lstlisting" => "[language=$1]", "$0", "listings";
    "minted" => "{$1}", "$0", "minted";
    "minipage" => "{$1\\linewidth}", "$0", "box";
    "frame" => "{$1}", "$0", "beamer";
    "columns" => "", "\\column{0.5\\textwidth}\n$0", "beamer";
    "block" => "{$1}", "$0", "beamer";
    "theorem" => "", "$0", "amsthm";
    "lemma" => "", "$0", "amsthm";
    "proof" => "", "$0", "amsthm";
    "definition" => "", "$0", "amsthm";
    "corollary" => "", "$0", "amsthm";
    "proposition" => "", "$0", "amsthm";
    "remark" => "", "$0", "amsthm";
    "example" => "", "$0", "amsthm";
    "tikzpicture" => "", "$0", "TikZ";
    "subfigure" => "{$1\\linewidth}", "$0", "subcaption";
    "thebibliography" => "{$1}", "\\bibitem{$2} $0", "bibliography";
};

/// Expands a snippet: removes `$n` markers and returns the text plus the cursor byte offset
/// within it (at `$1` if present, else `$0`, else the end).
pub fn expand_snippet(snippet: &str) -> (String, usize) {
    let mut out = String::with_capacity(snippet.len());
    let mut first: Option<usize> = None;
    let mut zero: Option<usize> = None;
    let mut chars = snippet.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$'
            && let Some(&d) = chars.peek()
            && d.is_ascii_digit()
        {
            chars.next();
            if d == '0' {
                zero.get_or_insert(out.len());
            } else if d == '1' {
                first.get_or_insert(out.len());
            }
            continue;
        }
        out.push(c);
    }
    let cursor = first.or(zero).unwrap_or(out.len());
    (out, cursor)
}

/// The text inserted when completing environment `name` after `\begin{`: the rest of the
/// `\begin` line, the body and the matching `\end`, indented by `indent`. Returns text and the
/// cursor offset within it.
pub fn environment_insertion(name: &str, indent: &str) -> (String, usize) {
    let info = ENVIRONMENTS.iter().find(|e| e.name == name);
    let args = info.map_or("", |e| e.arguments);
    let body = info.map_or("$0", |e| e.body);
    let body_indented: String = body
        .lines()
        .map(|l| format!("{indent}  {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let snippet = format!("{name}}}{args}\n{body_indented}\n{indent}\\end{{{name}}}");
    expand_snippet(&snippet)
}

/// What accepting a completion inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accept<'a> {
    /// A command snippet (after the backslash), e.g. `section{$1}`.
    Command(&'a str),
    /// An environment name after `\begin{` or `\end{`.
    Environment(&'a str),
    /// A key inside braces (label, citation, file).
    Key(&'a str),
}

/// A text edit: replace bytes `start..end` with `insert`; the caret goes to byte `caret` of
/// the new text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionEdit {
    /// First replaced byte.
    pub start: usize,
    /// End of the replaced bytes.
    pub end: usize,
    /// Replacement.
    pub insert: String,
    /// Caret, as a byte offset in the new text.
    pub caret: usize,
}

/// The edit that accepts a completion typed from byte `start` to the caret (byte `caret`) of
/// `text`. The rest of the word after the caret is replaced too (re-completing mid-word), and
/// braces already there are reused instead of doubled: `\begin{equ|}` (as left by completing
/// `\begin`) becomes one `\begin{equation}`, `\textb|{x}` becomes `\textbf{x}`.
pub fn completion_edit(
    text: &str,
    start: usize,
    caret: usize,
    accept: Accept<'_>,
) -> CompletionEdit {
    let floor = |mut i: usize| {
        i = i.min(text.len());
        while i > 0 && !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let caret = floor(caret);
    let start = floor(start.min(caret));
    let before = text.get(..start).unwrap_or("");
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text
        .get(line_start..)
        .unwrap_or("")
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    // The rest of the word after the caret.
    let rest: usize = text
        .get(caret..)
        .unwrap_or("")
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.' | '/'))
        .map(char::len_utf8)
        .sum();
    let mut end = caret.saturating_add(rest).min(text.len());
    let next = text.get(end..).and_then(|r| r.chars().next());
    let (insert, cursor) = match accept {
        Accept::Command(snippet) => {
            let (full, cursor) = expand_snippet(snippet);
            // Arguments already typed: complete only the name.
            let name_len = full.find(['{', '[']).unwrap_or(full.len());
            if matches!(next, Some('{' | '[')) && name_len < full.len() {
                let name = full.get(..name_len).unwrap_or(&full).to_string();
                let n = name.len();
                (name, n)
            } else {
                (full, cursor)
            }
        }
        Accept::Environment(name) => {
            if next == Some('}') {
                // Reuse the closing brace that is already there.
                end += 1;
            }
            if before.trim_end_matches('{').ends_with("\\begin") {
                environment_insertion(name, &indent)
            } else {
                let s = format!("{name}}}");
                let n = s.len();
                (s, n)
            }
        }
        Accept::Key(key) => {
            if next == Some('}') {
                // Keep the caret inside the braces (another key may follow after a comma).
                (key.to_string(), key.len())
            } else {
                let s = format!("{key}}}");
                let n = s.len();
                (s, n)
            }
        }
    };
    CompletionEdit {
        start,
        end,
        caret: start + cursor,
        insert,
    }
}

/// Case-insensitive fuzzy rank of `candidate` against `prefix`: `Some(score)` if it matches,
/// lower is better (exact prefix beats substring beats subsequence).
pub fn match_score(candidate: &str, prefix: &str) -> Option<u32> {
    if prefix.is_empty() {
        return Some(1000);
    }
    if candidate.starts_with(prefix) {
        return Some(u32::try_from(candidate.len()).unwrap_or(999).min(999));
    }
    let lc = candidate.to_ascii_lowercase();
    let lp = prefix.to_ascii_lowercase();
    if lc.starts_with(&lp) {
        return Some(1000 + u32::try_from(candidate.len()).unwrap_or(999).min(999));
    }
    if lc.contains(&lp) {
        return Some(2000 + u32::try_from(candidate.len()).unwrap_or(999).min(999));
    }
    let mut it = lc.chars();
    if lp.chars().all(|p| it.any(|c| c == p)) {
        return Some(3000 + u32::try_from(candidate.len()).unwrap_or(999).min(999));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(text_with_cursor: &str) -> CompletionContext {
        let cursor = text_with_cursor.find('|').unwrap();
        let text = text_with_cursor.replace('|', "");
        context_at(&text, cursor)
    }

    #[test]
    fn command_context() {
        assert_eq!(
            ctx("Hello \\tex|"),
            CompletionContext::Command {
                prefix: "tex".into(),
                start: 6
            }
        );
        assert_eq!(
            ctx("\\|"),
            CompletionContext::Command {
                prefix: String::new(),
                start: 0
            }
        );
        // `\\` is a line break, not a command start.
        assert_eq!(ctx("a \\\\|"), CompletionContext::None);
        assert_eq!(ctx("% \\tex|"), CompletionContext::None);
    }

    #[test]
    fn argument_contexts() {
        assert_eq!(
            ctx("\\begin{ite|"),
            CompletionContext::Environment {
                prefix: "ite".into(),
                start: 7
            }
        );
        assert_eq!(
            ctx("see \\eqref{eq:|"),
            CompletionContext::Reference {
                prefix: "eq:".into(),
                start: 11
            }
        );
        assert_eq!(
            ctx("\\cite[p.~3]{knuth, lam|"),
            CompletionContext::Citation {
                prefix: "lam".into(),
                start: 19
            }
        );
        assert!(matches!(
            ctx("\\includegraphics[width=3cm]{figs/|"),
            CompletionContext::File { ref prefix, .. } if prefix == "figs/"
        ));
        assert_eq!(ctx("\\textbf{bo|"), CompletionContext::None);
        assert_eq!(ctx("\\ref{a} b|"), CompletionContext::None);
    }

    #[test]
    fn snippets() {
        assert_eq!(expand_snippet("frac{$1}{$2}"), ("frac{}{}".into(), 5));
        assert_eq!(expand_snippet("item $0"), ("item ".into(), 5));
        assert_eq!(expand_snippet("maketitle"), ("maketitle".into(), 9));
        let (text, cur) = environment_insertion("itemize", "");
        assert_eq!(text, "itemize}\n  \\item \n\\end{itemize}");
        assert_eq!(&text[..cur], "itemize}\n  \\item ");
    }

    #[test]
    fn ranking() {
        assert!(match_score("section", "sec") < match_score("subsection", "sec"));
        assert!(match_score("subsection", "sec").is_some());
        assert_eq!(match_score("alpha", "xyz"), None);
    }

    #[test]
    fn catalogue_is_sane() {
        for c in COMMANDS {
            assert!(
                !c.name.is_empty() && c.snippet.starts_with(c.name),
                "{}",
                c.name
            );
        }
        let mut names: Vec<_> = COMMANDS.iter().map(|c| c.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate command in catalogue");
    }

    #[test]
    fn odd_cursors_never_panic() {
        let text = "中文\\section{标题}";
        for i in 0..=text.len() + 2 {
            let _ = context_at(text, i);
        }
    }

    /// Applies `completion_edit` to text with `|` at the caret and `^` at the start of the typed
    /// word; returns the new text with `|` at the new caret.
    fn accept(t: &str, a: Accept<'_>) -> String {
        let start = t.find('^').unwrap();
        let t = t.replace('^', "");
        let caret = t.find('|').unwrap();
        let t = t.replace('|', "");
        let e = completion_edit(&t, start, caret, a);
        let mut out = String::new();
        out.push_str(&t[..e.start]);
        out.push_str(&e.insert[..e.caret - e.start]);
        out.push('|');
        out.push_str(&e.insert[e.caret - e.start..]);
        out.push_str(&t[e.end..]);
        out
    }

    #[test]
    fn completion_reuses_existing_braces() {
        // \begin completed to `\begin{|}`, then the environment: no stray `}` after \end.
        assert_eq!(
            accept("\\begin{^equ|}\n", Accept::Environment("equation")),
            "\\begin{equation}\n  |\n\\end{equation}\n"
        );
        assert_eq!(
            accept("\\begin{^equ|\n", Accept::Environment("equation")),
            "\\begin{equation}\n  |\n\\end{equation}\n"
        );
        assert_eq!(
            accept("\\end{^equ|}", Accept::Environment("equation")),
            "\\end{equation}|"
        );
        assert_eq!(
            accept("\\end{^equ|", Accept::Environment("equation")),
            "\\end{equation}|"
        );
        // Mid-word re-completion replaces the rest of the word.
        assert_eq!(
            accept("\\end{^equ|ation}", Accept::Environment("equation")),
            "\\end{equation}|"
        );
        assert_eq!(
            accept("\\ref{^sec:in|}", Accept::Key("sec:intro")),
            "\\ref{sec:intro|}"
        );
        assert_eq!(
            accept("\\ref{^sec:in|", Accept::Key("sec:intro")),
            "\\ref{sec:intro}|"
        );
        assert_eq!(
            accept("\\input{^sec|}", Accept::Key("sections/intro")),
            "\\input{sections/intro|}"
        );
        // Commands: snippet when nothing follows, name only when the arguments are there.
        assert_eq!(
            accept("\\^sec|", Accept::Command("section{$1}")),
            "\\section{|}"
        );
        assert_eq!(
            accept("\\^textb|{x}", Accept::Command("textbf{$1}")),
            "\\textbf|{x}"
        );
        assert_eq!(
            accept("\\^beg|", Accept::Command("begin{$1}")),
            "\\begin{|}"
        );
        // Indentation carries over to the \end line.
        assert_eq!(
            accept("  \\begin{^ite|}", Accept::Environment("itemize")),
            "  \\begin{itemize}\n    \\item |\n  \\end{itemize}"
        );
    }

    #[test]
    fn completion_edit_never_panics() {
        let t = "中文\\begin{eq}";
        for s in 0..=t.len() + 2 {
            for c in 0..=t.len() + 2 {
                for a in [
                    Accept::Environment("equation"),
                    Accept::Key("k"),
                    Accept::Command("frac{$1}{$2}"),
                ] {
                    let e = completion_edit(t, s, c, a);
                    assert!(e.start <= e.end && e.caret >= e.start);
                }
            }
        }
    }
}
